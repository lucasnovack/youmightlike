//! Music domain recommendation extractor (artists and tracks)
//! with multi-seed fairness, hub-degree normalization, and confluence prioritization.

use super::super::context::{is_steam_noise_tag, RecContext};
use crate::models::{
    is_tag_relation, lookup_artist_meta, lookup_seed_top_tracks, norm_track_title, parse_node_key,
    resolve_artist_track_and_collab, GraphLinkPayload, GraphNodePayload, TopTrackRow,
};
use crate::ppr::top_k_from_scores;
use duckdb::params;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
struct CandidateArtist {
    nkey: String,
    nid: i64,
    aname: String,
    country: Option<String>,
    boosted_score: f32,
    connected_seeds: Vec<String>,
    primary_seed: Option<String>,
    full_reason: String,
    best_bridge: Option<(String, String, String, f32)>,
}

#[derive(Clone)]
struct TrackRecItem {
    artist_id: i64,
    artist_name: String,
    score: f32,
    track: TopTrackRow,
    reason: String,
    primary_seed: Option<String>,
    collab_seed_aid: Option<i64>,
    collab_seed_name: Option<String>,
}

pub fn extract_music(
    ctx: &RecContext,
    artist_quota: usize,
    track_quota: usize,
    results: &mut Vec<Value>,
    graph_nodes: &mut Vec<GraphNodePayload>,
    graph_links: &mut Vec<GraphLinkPayload>,
) {
    if artist_quota == 0 && track_quota == 0 {
        return;
    }

    let mut recommended_artist_tracks: Vec<TrackRecItem> = Vec::new();

    let mut seen_artist_names: HashSet<String> = ctx
        .seed_artist_names
        .values()
        .map(|s| s.trim().to_lowercase())
        .collect();
    if seen_artist_names.contains("kanye west") || seen_artist_names.contains("ye") {
        seen_artist_names.insert("kanye west".to_string());
        seen_artist_names.insert("ye".to_string());
    }

    let seed_artist_keys: Vec<String> = ctx
        .active_seed_set
        .iter()
        .filter(|s| s.starts_with("artist:"))
        .cloned()
        .collect();
    let has_music_seeds =
        !seed_artist_keys.is_empty() || ctx.active_seed_set.iter().any(|s| s.starts_with("track:"));

    // Pre-cache seed album titles for fast collaborative track discovery
    let mut seed_album_titles: HashMap<String, (i64, String)> = HashMap::new();
    for s_key in &seed_artist_keys {
        if let Some(("artist", s_aid)) = parse_node_key(s_key) {
            let s_name = ctx
                .seed_artist_names
                .get(s_key)
                .cloned()
                .unwrap_or_else(|| s_key.clone());
            if let Ok(mut stmt) = ctx
                .conn
                .prepare("SELECT title FROM music_albums WHERE artist_id = ?")
            {
                if let Ok(rows) = stmt.query_map(params![s_aid], |r| r.get::<_, Option<String>>(0))
                {
                    for r in rows.flatten().flatten() {
                        let norm = r.trim().to_lowercase();
                        if norm.len() >= 3
                            && norm != "single"
                            && norm != "ep"
                            && norm != "greatest hits"
                        {
                            seed_album_titles.insert(norm, (s_aid, s_name.clone()));
                        }
                    }
                }
            }
        }
    }

    // Pre-compute direct similar peer map and seed degree for all active music seeds
    let mut seed_peer_maps: HashMap<String, HashMap<String, f32>> = HashMap::new();
    let mut seed_degrees: HashMap<String, usize> = HashMap::new();
    for s_key in &seed_artist_keys {
        let edges = ctx.state.graph.out_edges_by_key(s_key);
        seed_degrees.insert(s_key.clone(), edges.len());
        let mut peers = HashMap::new();
        for e in edges {
            if e.relation_type == "similar_to" {
                peers.insert(e.target_key.to_string(), e.base_weight);
            }
        }
        seed_peer_maps.insert(s_key.clone(), peers);
    }

    let max_ost_composers = if has_music_seeds {
        1usize
    } else {
        (artist_quota / 2).max(2)
    };
    let mut ost_composers_added = 0usize;
    let mut ost_games_covered: HashSet<i64> = HashSet::new();

    let top_diffused_artists = top_k_from_scores(
        &ctx.state.graph,
        ctx.p,
        ctx.exclude_ids,
        "artist:",
        Some(300),
    );

    let mut candidate_pool: Vec<CandidateArtist> = Vec::new();

    for item in &top_diffused_artists {
        if item.score < ctx.min_rec_score * 0.15 {
            continue;
        }
        let nkey = &item.node_key;
        if ctx.seed_set.contains(nkey) {
            continue;
        }
        let Some(("artist", nid)) = parse_node_key(nkey) else {
            continue;
        };

        let Some((aname, country)) = lookup_artist_meta(ctx.conn, nid) else {
            continue;
        };
        let aname_norm = aname.trim().to_lowercase().replace('’', "'").replace(
            ['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}'],
            "-",
        );
        if seen_artist_names.contains(&aname_norm) {
            continue;
        }

        let out_edges = ctx.state.graph.out_edges_by_key(nkey);
        let mut connected_seeds: Vec<String> = Vec::new();
        let mut seed_similarities: HashMap<String, f32> = HashMap::new();
        let mut shared_tags: Vec<String> = Vec::new();
        let mut best_artist_bridge: Option<(String, String, String, f32)> = None;
        let mut matched_seed_game: Option<i64> = None;

        for e in &out_edges {
            if (e.relation_type == "soundtrack_composer" || e.relation_type == "soundtrack_for")
                && ctx
                    .seed_games
                    .contains(&parse_node_key(e.target_key).map(|(_, id)| id).unwrap_or(0))
            {
                if let Some(("game", gid)) = parse_node_key(e.target_key) {
                    if let Some(g) = ctx.state.catalog.games.get(&gid) {
                        if matched_seed_game.is_none() {
                            matched_seed_game = Some(gid);
                        }
                        if best_artist_bridge.is_none() {
                            best_artist_bridge = Some((
                                e.target_key.to_string(),
                                g.title.clone(),
                                "game".to_string(),
                                e.base_weight,
                            ));
                        }
                    }
                }
            } else if e.relation_type == "similar_to" && ctx.active_seed_set.contains(e.target_key)
            {
                if !connected_seeds.contains(&e.target_key.to_string()) {
                    connected_seeds.push(e.target_key.to_string());
                }
                seed_similarities
                    .entry(e.target_key.to_string())
                    .or_insert(e.base_weight);
                if best_artist_bridge.is_none() {
                    let s_name = ctx
                        .seed_artist_names
                        .get(e.target_key)
                        .cloned()
                        .unwrap_or_else(|| e.target_key.to_string());
                    best_artist_bridge = Some((
                        e.target_key.to_string(),
                        s_name,
                        "artist".to_string(),
                        e.base_weight,
                    ));
                }
            } else if is_tag_relation(e.relation_type) && e.target_key.starts_with("keyword:") {
                if let Some(("keyword", kid)) = parse_node_key(e.target_key) {
                    if ctx.seed_tag_counts.contains_key(&kid) && shared_tags.len() < 4 {
                        if let Some(kname) = ctx.state.catalog.keywords.get(&kid) {
                            if !is_steam_noise_tag(kname.as_str()) {
                                let tfmt = format!("#{}", kname);
                                if !shared_tags.contains(&tfmt) {
                                    shared_tags.push(tfmt.clone());
                                }
                                if best_artist_bridge.is_none() {
                                    best_artist_bridge = Some((
                                        e.target_key.to_string(),
                                        tfmt,
                                        "keyword".to_string(),
                                        e.base_weight,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Also check if any seed artist points to this candidate via similar_to
        for (s_key, peers) in &seed_peer_maps {
            if let Some(&weight) = peers.get(nkey) {
                if !connected_seeds.contains(s_key) {
                    connected_seeds.push(s_key.clone());
                }
                seed_similarities.entry(s_key.clone()).or_insert(weight);
                if best_artist_bridge.is_none() {
                    let s_name = ctx
                        .seed_artist_names
                        .get(s_key)
                        .cloned()
                        .unwrap_or_else(|| s_key.clone());
                    best_artist_bridge =
                        Some((s_key.clone(), s_name, "artist".to_string(), weight));
                }
            }
        }

        let is_vgm_composer = out_edges.iter().any(|e| {
            e.relation_type == "soundtrack_for" || e.relation_type == "soundtrack_composer"
        });

        if is_vgm_composer && connected_seeds.is_empty() {
            if let Some(gid) = matched_seed_game {
                if ost_composers_added >= max_ost_composers || !ost_games_covered.insert(gid) {
                    continue;
                }
                ost_composers_added += 1;
            } else if has_music_seeds || !ctx.seed_games.is_empty() {
                continue;
            }
        }

        // Suppress isolated noise nodes unless they have direct similarity to an active seed
        if out_edges.len() < 4 && connected_seeds.is_empty() && !is_vgm_composer {
            continue;
        }

        // Confluence requirement for cross-domain music recommendations:
        // When jumping from non-music domains (movies, games, books, series) into music,
        // a candidate artist MUST have confluence (at least 2 shared thematic tags,
        // or a direct soundtrack/OST bridge). A single isolated tag (e.g. #time) is lexical noise.
        let shared_tag_count = shared_tags.len();
        if !has_music_seeds && !is_vgm_composer && connected_seeds.is_empty() {
            if shared_tag_count < 2 {
                continue;
            }
            let has_tracks: bool = ctx
                .conn
                .query_row(
                    "SELECT 1 FROM music_tracks WHERE artist_id = ? LIMIT 1",
                    params![nid],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if !has_tracks {
                continue;
            }
        }

        seen_artist_names.insert(aname_norm);

        // Explanation assembly
        let mut explanation_parts: Vec<String> = Vec::new();

        if let Some(gid) = matched_seed_game {
            if let Some(g) = ctx.state.catalog.games.get(&gid) {
                explanation_parts.push(if ctx.is_pt {
                    format!("Compositor da trilha de {}", g.title)
                } else {
                    format!("Soundtrack composer for {}", g.title)
                });
            }
        }

        if connected_seeds.len() >= 2 {
            let names: Vec<String> = connected_seeds
                .iter()
                .map(|s| {
                    ctx.seed_artist_names
                        .get(s)
                        .cloned()
                        .unwrap_or_else(|| s.clone())
                })
                .collect();
            explanation_parts.push(if ctx.is_pt {
                format!("Ponte de co-escuta no Last.fm entre {}", names.join(" e "))
            } else {
                format!(
                    "Last.fm co-listening bridge between {}",
                    names.join(" and ")
                )
            });
        } else if connected_seeds.len() == 1 {
            let s_key = &connected_seeds[0];
            let s_name = ctx
                .seed_artist_names
                .get(s_key)
                .cloned()
                .unwrap_or_else(|| s_key.clone());
            let pct_str = seed_similarities
                .get(s_key)
                .map(|&w| {
                    let p = ((w / 2.0).clamp(0.05, 1.0) * 100.0).round() as i32;
                    format!(" ({}% de sobreposição)", p)
                })
                .unwrap_or_default();
            let pct_str_en = seed_similarities
                .get(s_key)
                .map(|&w| {
                    let p = ((w / 2.0).clamp(0.05, 1.0) * 100.0).round() as i32;
                    format!(" ({}% overlap)", p)
                })
                .unwrap_or_default();

            explanation_parts.push(if ctx.is_pt {
                format!("Co-escuta no Last.fm com {}{}", s_name, pct_str)
            } else {
                format!("Last.fm co-listening with {}{}", s_name, pct_str_en)
            });
        }

        if !shared_tags.is_empty() {
            let tags_joined = shared_tags.join(", ");
            explanation_parts.push(if ctx.is_pt {
                format!("Tags: {}", tags_joined)
            } else {
                format!("Tags: {}", tags_joined)
            });
        }

        let default_reason = if let Some((_, ref b_label, ref b_type, _)) = best_artist_bridge {
            if b_type == "keyword" {
                if ctx.is_pt {
                    format!("Convergência estética via {}", b_label)
                } else {
                    format!("Aesthetic convergence via {}", b_label)
                }
            } else if ctx.is_pt {
                format!("Ressonância de cluster com {}", b_label)
            } else {
                format!("Cluster resonance with {}", b_label)
            }
        } else if ctx.is_pt {
            "Afinidade e proximidade no grafo musical".to_string()
        } else {
            "Graph diffusion affinity and proximity".to_string()
        };
        let full_reason = if explanation_parts.is_empty() {
            default_reason
        } else {
            explanation_parts.join(" • ")
        };

        // Score weighting with multi-seed confluence boost, dead-end damping, and hub normalization
        let cand_deg = out_edges.len();

        let mut max_sim_weight = 0.0f32;
        for e in &out_edges {
            if e.relation_type == "similar_to"
                && ctx.active_seed_set.contains(e.target_key)
                && e.base_weight > max_sim_weight
            {
                max_sim_weight = e.base_weight;
            }
        }
        for peers in seed_peer_maps.values() {
            if let Some(&w) = peers.get(nkey) {
                if w > max_sim_weight {
                    max_sim_weight = w;
                }
            }
        }

        let mut boost = 1.0f32;
        if connected_seeds.len() >= 2 {
            // Highly prized confluence node bridging multiple seeds
            boost *= 2.5;
        } else if connected_seeds.len() == 1 {
            let s_key = &connected_seeds[0];
            let deg = seed_degrees.get(s_key).copied().unwrap_or(300);
            let deg_factor = ((deg as f32) / 300.0).sqrt().clamp(1.0, 2.3);
            boost *= deg_factor;
        }

        // Multi-tag thematic confluence boost
        if !has_music_seeds && !is_vgm_composer {
            boost *= 1.0 + (shared_tag_count as f32).powi(2) * 3.0;

            // Popular acclaim weighting for cross-media discovery:
            // Favors recognized seminal works over obscure bedroom uploads
            let max_pop: f32 = ctx
                .conn
                .query_row(
                    "SELECT COALESCE(MAX(popularity), 0) FROM music_tracks WHERE artist_id = ?",
                    params![nid],
                    |r| r.get(0),
                )
                .unwrap_or(0.0);
            let pop_factor = if max_pop < 15.0 {
                0.25
            } else if max_pop < 40.0 {
                0.70
            } else {
                1.0 + (max_pop / 50.0).min(2.0) * 0.5
            };
            boost *= pop_factor;
        } else if shared_tag_count > 0 {
            boost *= 1.0 + (shared_tag_count as f32) * 0.4;
        }

        // Direct high-confidence peer boost (e.g. Kanye <-> Jay-Z has weight 0.74)
        if max_sim_weight > 0.0 {
            boost *= 1.0 + max_sim_weight * 2.2;
        }

        // Dead-End Damping: single-edge dead ends should NOT surpass major collaborative peers
        if cand_deg <= 2 {
            boost *= 0.25;
        } else if cand_deg <= 5 {
            boost *= 0.65;
        }

        // Hub degree normalization: compensate large catalog artists whose probability gets diluted across 20+ edges
        if cand_deg >= 20 {
            let hub_factor = ((cand_deg as f32) / 20.0).powf(0.35).clamp(1.0, 2.5);
            boost *= hub_factor;
        }

        let boosted_score = item.score * boost;

        let primary_seed = if connected_seeds.len() == 1 {
            Some(connected_seeds[0].clone())
        } else {
            None
        };

        candidate_pool.push(CandidateArtist {
            nkey: nkey.clone(),
            nid,
            aname,
            country,
            boosted_score,
            connected_seeds,
            primary_seed,
            full_reason,
            best_bridge: best_artist_bridge,
        });
    }

    // -----------------------------------------------------------------------
    // Balanced Multi-Seed Artist Selection (Round-Robin Fair Draft)
    // -----------------------------------------------------------------------
    let mut chosen_candidates: Vec<CandidateArtist> = Vec::new();
    let mut chosen_keys: HashSet<String> = HashSet::new();

    if seed_artist_keys.len() > 1 && artist_quota > 0 {
        // Pass 1: Prioritize multi-seed confluence candidates (e.g. Lady Gaga bridging Kanye & Phantogram)
        let mut confluence_cands: Vec<&CandidateArtist> = candidate_pool
            .iter()
            .filter(|c| c.connected_seeds.len() >= 2)
            .collect();
        confluence_cands.sort_by(|a, b| {
            b.boosted_score
                .partial_cmp(&a.boosted_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let max_confluence = if artist_quota >= 6 { 2 } else { 1 };
        for c in confluence_cands.into_iter().take(max_confluence) {
            if chosen_candidates.len() >= artist_quota {
                break;
            }
            if chosen_keys.insert(c.nkey.clone()) {
                chosen_candidates.push(c.clone());
            }
        }

        // Pass 2: Round-Robin fair draft across all active seed artists
        let mut seed_queues: HashMap<String, Vec<&CandidateArtist>> = HashMap::new();
        for s_key in &seed_artist_keys {
            let mut list: Vec<&CandidateArtist> = candidate_pool
                .iter()
                .filter(|c| {
                    (c.primary_seed.as_ref() == Some(s_key) || c.connected_seeds.contains(s_key))
                        && !chosen_keys.contains(&c.nkey)
                })
                .collect();
            list.sort_by(|a, b| {
                b.boosted_score
                    .partial_cmp(&a.boosted_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            seed_queues.insert(s_key.clone(), list);
        }

        let mut round = 0;
        let mut made_progress = true;
        while chosen_candidates.len() < artist_quota && made_progress {
            made_progress = false;
            for s_key in &seed_artist_keys {
                if chosen_candidates.len() >= artist_quota {
                    break;
                }
                if let Some(queue) = seed_queues.get_mut(s_key) {
                    while !queue.is_empty() {
                        let cand = queue.remove(0);
                        if chosen_keys.insert(cand.nkey.clone()) {
                            chosen_candidates.push(cand.clone());
                            made_progress = true;
                            break;
                        }
                    }
                }
            }
            round += 1;
            if round > 10 {
                break;
            }
        }

        // Pass 3: Fill remaining quota with highest boosted score candidates
        let mut remaining_cands: Vec<&CandidateArtist> = candidate_pool
            .iter()
            .filter(|c| !chosen_keys.contains(&c.nkey))
            .collect();
        remaining_cands.sort_by(|a, b| {
            b.boosted_score
                .partial_cmp(&a.boosted_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for c in remaining_cands {
            if chosen_candidates.len() >= artist_quota {
                break;
            }
            if chosen_keys.insert(c.nkey.clone()) {
                chosen_candidates.push(c.clone());
            }
        }
    } else if artist_quota > 0 {
        let mut sorted_cands: Vec<&CandidateArtist> = candidate_pool.iter().collect();
        sorted_cands.sort_by(|a, b| {
            b.boosted_score
                .partial_cmp(&a.boosted_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for c in sorted_cands.into_iter().take(artist_quota) {
            if chosen_keys.insert(c.nkey.clone()) {
                chosen_candidates.push(c.clone());
            }
        }
    }

    // Materialize recommended artists
    for c in &chosen_candidates {
        // Resolve track and joint collaboration exclusively for this chosen artist
        let (resolved_track, collab_info) =
            resolve_artist_track_and_collab(ctx.conn, c.nid, &seed_album_titles)
                .map(|(t, clb)| (Some(t), clb))
                .unwrap_or((None, None));

        let sub = c
            .country
            .as_ref()
            .map(|ct| {
                if ctx.is_pt {
                    format!("Artista • {}", ct)
                } else {
                    format!("Artist • {}", ct)
                }
            })
            .unwrap_or_else(|| {
                if ctx.is_pt {
                    "Artista".to_string()
                } else {
                    "Artist".to_string()
                }
            });

        let top_track_val = resolved_track.as_ref().map(|t| {
            json!({
                "title": t.title,
                "popularity": t.popularity.unwrap_or(50),
            })
        });

        let mut final_reason = c.full_reason.clone();
        if let Some((_, ref s_name)) = collab_info {
            let alb = resolved_track
                .as_ref()
                .and_then(|t| t.album_title.as_deref())
                .unwrap_or("compartilhado");
            let collab_desc = if ctx.is_pt {
                format!("Colaboração com {} no álbum {}", s_name, alb)
            } else {
                format!("Collaboration with {} on album {}", s_name, alb)
            };
            final_reason = format!("{} • {}", collab_desc, final_reason);
        }

        results.push(json!({
            "id": c.nkey,
            "title": c.aname,
            "type": "artist",
            "subtitle": sub,
            "score": ((c.boosted_score as f64) * 1000.0 * 100.0).round() / 100.0,
            "top_track": top_track_val,
            "reason": final_reason,
        }));

        graph_nodes.push(GraphNodePayload {
            id: c.nkey.clone(),
            label: c.aname.clone(),
            node_type: "artist".to_string(),
            val: 12.0,
            tags: Vec::new(),
            desc: String::new(),
            metadata: None,
        });

        if let Some((b_key, b_label, b_type, b_weight)) = &c.best_bridge {
            if !graph_nodes.iter().any(|n| n.id == *b_key) {
                graph_nodes.push(GraphNodePayload {
                    id: b_key.clone(),
                    label: b_label.clone(),
                    node_type: b_type.clone(),
                    val: 9.0,
                    tags: Vec::new(),
                    desc: String::new(),
                    metadata: None,
                });
            }
            graph_links.push(GraphLinkPayload {
                source: c.nkey.clone(),
                target: b_key.clone(),
                label: if ctx.is_pt {
                    "conectado_a".to_string()
                } else {
                    "connected_to".to_string()
                },
                weight: *b_weight,
            });
        }

        // Link collaborative artist directly to the seed artist they collaborated with
        if let Some((collab_aid, _)) = collab_info {
            let seed_key = format!("artist:{}", collab_aid);
            if !graph_links.iter().any(|l| {
                (l.source == c.nkey && l.target == seed_key)
                    || (l.source == seed_key && l.target == c.nkey)
            }) {
                graph_links.push(GraphLinkPayload {
                    source: c.nkey.clone(),
                    target: seed_key,
                    label: if ctx.is_pt {
                        "colaborou_com".to_string()
                    } else {
                        "collaborated_with".to_string()
                    },
                    weight: 3.0,
                });
            }
        }

        if let Some(ref trk) = resolved_track {
            let primary = c
                .primary_seed
                .clone()
                .or_else(|| c.connected_seeds.first().cloned());
            recommended_artist_tracks.push(TrackRecItem {
                artist_id: c.nid,
                artist_name: c.aname.clone(),
                score: c.boosted_score,
                track: trk.clone(),
                reason: final_reason,
                primary_seed: primary,
                collab_seed_aid: collab_info.as_ref().map(|(id, _)| *id),
                collab_seed_name: collab_info.as_ref().map(|(_, name)| name.clone()),
            });
        }
    }

    // -----------------------------------------------------------------------
    // Balanced Multi-Seed Track Selection (Seed Works + Joint Collabs + Peers)
    // -----------------------------------------------------------------------
    if track_quota > 0 {
        let mut track_recs_added = 0usize;
        let mut seen_rec_track_ids: HashSet<i64> = HashSet::new();
        let mut seen_rec_track_artists: HashSet<i64> = HashSet::new();
        let mut seen_rec_track_titles: HashSet<String> = HashSet::new();

        // Initialize with all track seed titles so no seed work is ever recommended back to the user
        for s_key in ctx.seed_set.iter().filter(|s| s.starts_with("track:")) {
            if let Some(("track", tid)) = parse_node_key(s_key) {
                seen_rec_track_ids.insert(tid);
                if let Ok(stitle) = ctx.conn.query_row(
                    "SELECT title FROM music_tracks WHERE id = ?",
                    params![tid],
                    |r| r.get::<_, String>(0),
                ) {
                    let norm = norm_track_title(&stitle);
                    if !norm.is_empty() {
                        seen_rec_track_titles.insert(norm);
                    }
                }
            }
        }

        // 0A. Direct track-to-track recommendations from active seed tracks!
        // (Album sibling tracks + Audio vibe affinity radio)
        let seed_track_keys: Vec<String> = ctx
            .seed_set
            .iter()
            .filter(|s| s.starts_with("track:"))
            .cloned()
            .collect();

        let mut seen_rec_album_counts: HashMap<i64, usize> = HashMap::new();

        // Reserve track recommendation slots for active seed artists that don't have track seeds (e.g. Kanye West)
        let artists_without_track_seeds: Vec<String> = seed_artist_keys
            .iter()
            .filter(|ak| {
                if let Some(("artist", aid)) = parse_node_key(ak) {
                    !seed_track_keys.iter().any(|tk| {
                        if let Some(("track", tid)) = parse_node_key(tk) {
                            ctx.conn
                                .query_row(
                                    "SELECT artist_id FROM music_tracks WHERE id = ?",
                                    params![tid],
                                    |r| r.get::<_, i64>(0),
                                )
                                .map(|a| a == aid)
                                .unwrap_or(false)
                        } else {
                            false
                        }
                    })
                } else {
                    false
                }
            })
            .cloned()
            .collect();

        let reserved_slots = (artists_without_track_seeds.len() * 2).min(track_quota / 2);
        let max_from_step_0a = track_quota.saturating_sub(reserved_slots).max(2);

        for s_key in &seed_track_keys {
            if track_recs_added >= max_from_step_0a {
                break;
            }
            let Some(("track", s_tid)) = parse_node_key(s_key) else {
                continue;
            };

            let meta_res: Result<(String, i64, Option<i64>, Option<f64>), _> = ctx.conn.query_row(
                "SELECT t.title, t.artist_id, t.album_id, t.tempo
                 FROM music_tracks t
                 WHERE t.id = ?",
                params![s_tid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            );

            if let Ok((seed_title, seed_aid, seed_alb_id, seed_tempo)) = meta_res {
                // 1. Same Album Tracks (e.g. Watch The Throne -> Ni**as In Paris, Otis, Who Gon Stop Me)
                if let Some(alb_id) = seed_alb_id {
                    let alb_count = seen_rec_album_counts.entry(alb_id).or_insert(0);
                    if *alb_count >= 2 {
                        continue;
                    }

                    let album_sql = "
                        SELECT t.id, t.title, t.popularity, t.tempo, ma.name, ma.id, alb.title
                        FROM music_tracks t
                        JOIN music_artists ma ON ma.id = t.artist_id
                        JOIN music_albums alb ON alb.id = t.album_id
                        WHERE t.album_id = ?
                          AND t.id != ?
                        ORDER BY t.popularity DESC NULLS LAST
                        LIMIT 2
                    ";
                    if let Ok(mut stmt) = ctx.conn.prepare(album_sql) {
                        if let Ok(mut rows) = stmt.query(params![alb_id, s_tid]) {
                            while let Ok(Some(r)) = rows.next() {
                                if track_recs_added >= max_from_step_0a
                                    || *seen_rec_album_counts.entry(alb_id).or_insert(0) >= 2
                                {
                                    break;
                                }
                                let (Ok(r_id), Ok(r_title), Ok(r_aname), Ok(r_aid), Ok(r_albtitle)) = (
                                    r.get::<_, i64>(0),
                                    r.get::<_, String>(1),
                                    r.get::<_, String>(4),
                                    r.get::<_, i64>(5),
                                    r.get::<_, String>(6),
                                ) else {
                                    continue;
                                };
                                let r_pop: Option<i32> = r.get(2).ok();
                                let _r_tempo: Option<f64> = r.get(3).ok();

                                let norm_t = norm_track_title(&r_title);
                                if norm_t.is_empty()
                                    || !seen_rec_track_titles.insert(norm_t)
                                    || !seen_rec_track_ids.insert(r_id)
                                    || ctx.seed_set.contains(&format!("track:{}", r_id))
                                {
                                    continue;
                                }

                                let t_key = format!("track:{}", r_id);
                                let art_key = format!("artist:{}", r_aid);

                                let trk_subtitle = if ctx.is_pt {
                                    format!("Música de {} • Álbum: {}", r_aname, r_albtitle)
                                } else {
                                    format!("Track by {} • Album: {}", r_aname, r_albtitle)
                                };

                                let trk_reason = if ctx.is_pt {
                                    format!("Mesmo álbum de {} ({})", seed_title, r_albtitle)
                                } else {
                                    format!("Same album as {} ({})", seed_title, r_albtitle)
                                };

                                results.push(json!({
                                    "id": t_key,
                                    "title": r_title,
                                    "type": "track",
                                    "subtitle": trk_subtitle,
                                    "score": 10.0,
                                    "top_track": {
                                        "title": r_title,
                                        "popularity": r_pop.unwrap_or(75),
                                    },
                                    "reason": trk_reason,
                                }));

                                graph_nodes.push(GraphNodePayload {
                                    id: t_key.clone(),
                                    label: r_title.clone(),
                                    node_type: "track".to_string(),
                                    val: 10.5,
                                    tags: Vec::new(),
                                    desc: String::new(),
                                    metadata: None,
                                });

                                graph_links.push(GraphLinkPayload {
                                    source: s_key.clone(),
                                    target: t_key.clone(),
                                    label: if ctx.is_pt {
                                        "mesmo_álbum".to_string()
                                    } else {
                                        "same_album".to_string()
                                    },
                                    weight: 3.5,
                                });

                                if !graph_nodes.iter().any(|n| n.id == art_key) {
                                    graph_nodes.push(GraphNodePayload {
                                        id: art_key.clone(),
                                        label: r_aname,
                                        node_type: "artist".to_string(),
                                        val: 12.0,
                                        tags: Vec::new(),
                                        desc: String::new(),
                                        metadata: None,
                                    });
                                }
                                if !graph_links.iter().any(|l| {
                                    (l.source == t_key && l.target == art_key)
                                        || (l.source == art_key && l.target == t_key)
                                }) {
                                    graph_links.push(GraphLinkPayload {
                                        source: t_key,
                                        target: art_key,
                                        label: "performed_by".to_string(),
                                        weight: 2.5,
                                    });
                                }

                                track_recs_added += 1;
                            }
                        }
                    }
                }

                // 2. Spotify Radio / Vibe Affinity (similar BPM audio features)
                if let Some(bpm) = seed_tempo {
                    if bpm > 0.0 && track_recs_added < track_quota {
                        let min_bpm = bpm - 12.0;
                        let max_bpm = bpm + 12.0;

                        let mut pool_aids: Vec<i64> = vec![seed_aid];

                        // Direct musical peers from artist_similarities
                        let sim_sql = "SELECT target_artist_id FROM artist_similarities WHERE source_artist_id = ?
                                       UNION
                                       SELECT source_artist_id FROM artist_similarities WHERE target_artist_id = ? LIMIT 10";
                        if let Ok(mut s_stmt) = ctx.conn.prepare(sim_sql) {
                            if let Ok(mut s_rows) = s_stmt.query([seed_aid, seed_aid]) {
                                while let Ok(Some(sr)) = s_rows.next() {
                                    if let Ok(aid) = sr.get::<_, i64>(0) {
                                        if !pool_aids.contains(&aid) {
                                            pool_aids.push(aid);
                                        }
                                    }
                                }
                            }
                        }

                        // Also include top high-affinity diffused artists (filtered by score threshold)
                        for &aid in ctx.top_diffused_artist_ids.iter().take(5) {
                            if !pool_aids.contains(&aid) {
                                pool_aids.push(aid);
                            }
                        }

                        let placeholders =
                            pool_aids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                        let vibe_sql = format!(
                            "SELECT t.id, t.title, t.popularity, t.tempo, ma.name, ma.id, alb.title
                             FROM music_tracks t
                             JOIN music_artists ma ON ma.id = t.artist_id
                             LEFT JOIN music_albums alb ON alb.id = t.album_id
                             WHERE t.artist_id IN ({})
                               AND t.id != ?
                               AND t.tempo BETWEEN ? AND ?
                             ORDER BY t.popularity DESC NULLS LAST
                             LIMIT 3",
                            placeholders
                        );

                        if let Ok(mut v_stmt) = ctx.conn.prepare(&vibe_sql) {
                            let mut params_vec: Vec<Box<dyn duckdb::ToSql>> = pool_aids
                                .into_iter()
                                .map(|id| Box::new(id) as Box<dyn duckdb::ToSql>)
                                .collect();
                            params_vec.push(Box::new(s_tid));
                            params_vec.push(Box::new(min_bpm));
                            params_vec.push(Box::new(max_bpm));
                            let params_ref: Vec<&dyn duckdb::ToSql> =
                                params_vec.iter().map(|b| &**b).collect();

                            if let Ok(mut v_rows) = v_stmt.query(&params_ref[..]) {
                                while let Ok(Some(r)) = v_rows.next() {
                                    if track_recs_added >= track_quota {
                                        break;
                                    }
                                    let (Ok(r_id), Ok(r_title), Ok(r_aname), Ok(r_aid)) = (
                                        r.get::<_, i64>(0),
                                        r.get::<_, String>(1),
                                        r.get::<_, String>(4),
                                        r.get::<_, i64>(5),
                                    ) else {
                                        continue;
                                    };
                                    let r_pop: Option<i32> = r.get(2).ok();
                                    let r_bpm: Option<f64> = r.get(3).ok();
                                    let r_albtitle: Option<String> = r.get(6).ok();

                                    let norm_t = norm_track_title(&r_title);
                                    if norm_t.is_empty()
                                        || !seen_rec_track_titles.insert(norm_t)
                                        || !seen_rec_track_ids.insert(r_id)
                                        || ctx.seed_set.contains(&format!("track:{}", r_id))
                                    {
                                        continue;
                                    }

                                    let t_key = format!("track:{}", r_id);
                                    let art_key = format!("artist:{}", r_aid);
                                    let bpm_display = r_bpm
                                        .map(|b| format!("~{} BPM", b.round() as i32))
                                        .unwrap_or_else(|| "BPM similar".to_string());

                                    let trk_subtitle = if let Some(ref alb) = r_albtitle {
                                        if ctx.is_pt {
                                            format!("Música de {} • {}", r_aname, alb)
                                        } else {
                                            format!("Track by {} • {}", r_aname, alb)
                                        }
                                    } else {
                                        if ctx.is_pt {
                                            format!("Música de {}", r_aname)
                                        } else {
                                            format!("Track by {}", r_aname)
                                        }
                                    };

                                    let trk_reason = if ctx.is_pt {
                                        format!(
                                            "Vibe sonora e andamento similar a {} ({})",
                                            seed_title, bpm_display
                                        )
                                    } else {
                                        format!(
                                            "Similar vibe and tempo to {} ({})",
                                            seed_title, bpm_display
                                        )
                                    };

                                    results.push(json!({
                                        "id": t_key,
                                        "title": r_title,
                                        "type": "track",
                                        "subtitle": trk_subtitle,
                                        "score": 9.2,
                                        "top_track": {
                                            "title": r_title,
                                            "popularity": r_pop.unwrap_or(65),
                                        },
                                        "reason": trk_reason,
                                    }));

                                    graph_nodes.push(GraphNodePayload {
                                        id: t_key.clone(),
                                        label: r_title.clone(),
                                        node_type: "track".to_string(),
                                        val: 10.0,
                                        tags: Vec::new(),
                                        desc: String::new(),
                                        metadata: None,
                                    });

                                    graph_links.push(GraphLinkPayload {
                                        source: s_key.clone(),
                                        target: t_key.clone(),
                                        label: if ctx.is_pt {
                                            "vibe_similar".to_string()
                                        } else {
                                            "similar_vibe".to_string()
                                        },
                                        weight: 2.8,
                                    });

                                    if !graph_nodes.iter().any(|n| n.id == art_key) {
                                        graph_nodes.push(GraphNodePayload {
                                            id: art_key.clone(),
                                            label: r_aname,
                                            node_type: "artist".to_string(),
                                            val: 12.0,
                                            tags: Vec::new(),
                                            desc: String::new(),
                                            metadata: None,
                                        });
                                    }
                                    if !graph_links.iter().any(|l| {
                                        (l.source == t_key && l.target == art_key)
                                            || (l.source == art_key && l.target == t_key)
                                    }) {
                                        graph_links.push(GraphLinkPayload {
                                            source: t_key,
                                            target: art_key,
                                            label: "performed_by".to_string(),
                                            weight: 2.5,
                                        });
                                    }

                                    track_recs_added += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        // 0. Seminal works from the active seed artists themselves!
        // When user chooses Kanye West or Phantogram, they expect to see their seminal works in the canvas and recs.
        if has_music_seeds {
            for s_key in &seed_artist_keys {
                if track_recs_added >= track_quota {
                    break;
                }
                let Some(("artist", s_aid)) = parse_node_key(s_key) else {
                    continue;
                };
                let s_name = ctx
                    .seed_artist_names
                    .get(s_key)
                    .cloned()
                    .unwrap_or_else(|| s_key.clone());

                let max_seed_tracks = if seed_artist_keys.len() == 1 { 3 } else { 2 };
                let seed_tracks = lookup_seed_top_tracks(ctx.conn, s_aid, max_seed_tracks);
                for trk in seed_tracks {
                    if track_recs_added >= track_quota {
                        break;
                    }
                    let norm_t = norm_track_title(&trk.title);
                    if norm_t.is_empty()
                        || !seen_rec_track_titles.insert(norm_t)
                        || !seen_rec_track_ids.insert(trk.track_id)
                        || ctx.seed_set.contains(&format!("track:{}", trk.track_id))
                    {
                        continue;
                    }
                    seen_rec_track_artists.insert(s_aid);

                    let t_key = format!("track:{}", trk.track_id);
                    let alb_str = trk.album_title.as_deref().unwrap_or("Discografia");
                    let trk_subtitle = if ctx.is_pt {
                        format!("Música de {} • Álbum: {}", s_name, alb_str)
                    } else {
                        format!("Track by {} • Album: {}", s_name, alb_str)
                    };
                    let trk_reason = if ctx.is_pt {
                        format!("Obra essencial do artista semente {}", s_name)
                    } else {
                        format!("Essential work of seed artist {}", s_name)
                    };

                    results.push(json!({
                        "id": t_key,
                        "title": trk.title,
                        "type": "track",
                        "subtitle": trk_subtitle,
                        "score": 9.5,
                        "top_track": {
                            "title": trk.title,
                            "popularity": trk.popularity.unwrap_or(75),
                        },
                        "reason": trk_reason,
                    }));

                    let art_key = format!("artist:{}", s_aid);
                    graph_nodes.push(GraphNodePayload {
                        id: t_key.clone(),
                        label: trk.title.clone(),
                        node_type: "track".to_string(),
                        val: 9.5,
                        tags: Vec::new(),
                        desc: String::new(),
                        metadata: None,
                    });
                    graph_links.push(GraphLinkPayload {
                        source: t_key,
                        target: art_key,
                        label: "performed_by".to_string(),
                        weight: 3.0,
                    });
                    track_recs_added += 1;
                }
            }
        }

        // 1. Joint collaborative works between recommended artists and seeds (e.g. JAY-Z & Kanye -> Watch The Throne)
        let mut collab_tracks: Vec<TrackRecItem> = recommended_artist_tracks
            .iter()
            .filter(|t| t.collab_seed_aid.is_some())
            .cloned()
            .collect();
        collab_tracks.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for c_item in collab_tracks {
            if track_recs_added >= track_quota {
                break;
            }
            let norm_t = norm_track_title(&c_item.track.title);
            if norm_t.is_empty()
                || !seen_rec_track_titles.insert(norm_t)
                || ctx
                    .seed_set
                    .contains(&format!("track:{}", c_item.track.track_id))
                || !seen_rec_track_ids.insert(c_item.track.track_id)
                || !seen_rec_track_artists.insert(c_item.artist_id)
            {
                continue;
            }

            let t_key = format!("track:{}", c_item.track.track_id);
            let alb_str = c_item.track.album_title.as_deref().unwrap_or("Colaboração");
            let trk_subtitle = if ctx.is_pt {
                format!("Música de {} • Álbum: {}", c_item.artist_name, alb_str)
            } else {
                format!("Track by {} • Album: {}", c_item.artist_name, alb_str)
            };
            let collab_name = c_item.collab_seed_name.as_deref().unwrap_or("artista");
            let trk_reason = if ctx.is_pt {
                format!(
                    "Colaboração histórica com {} no álbum {}",
                    collab_name, alb_str
                )
            } else {
                format!(
                    "Historic collaboration with {} on album {}",
                    collab_name, alb_str
                )
            };

            results.push(json!({
                "id": t_key,
                "title": c_item.track.title,
                "type": "track",
                "subtitle": trk_subtitle,
                "score": ((c_item.score as f64) * 980.0 * 100.0).round() / 100.0,
                "top_track": {
                    "title": c_item.track.title,
                    "popularity": c_item.track.popularity.unwrap_or(70),
                },
                "reason": trk_reason,
            }));

            let art_key = format!("artist:{}", c_item.artist_id);
            if !graph_nodes.iter().any(|n| n.id == art_key) {
                graph_nodes.push(GraphNodePayload {
                    id: art_key.clone(),
                    label: c_item.artist_name.clone(),
                    node_type: "artist".to_string(),
                    val: 11.5,
                    tags: Vec::new(),
                    desc: String::new(),
                    metadata: None,
                });
            }
            graph_nodes.push(GraphNodePayload {
                id: t_key.clone(),
                label: c_item.track.title.clone(),
                node_type: "track".to_string(),
                val: 10.5,
                tags: Vec::new(),
                desc: String::new(),
                metadata: None,
            });
            graph_links.push(GraphLinkPayload {
                source: t_key.clone(),
                target: art_key,
                label: "performed_by".to_string(),
                weight: 2.8,
            });
            if let Some(collab_aid) = c_item.collab_seed_aid {
                let seed_art_key = format!("artist:{}", collab_aid);
                graph_links.push(GraphLinkPayload {
                    source: t_key,
                    target: seed_art_key,
                    label: if ctx.is_pt {
                        "colaborou_com".to_string()
                    } else {
                        "collaborated_with".to_string()
                    },
                    weight: 2.8,
                });
            }
            track_recs_added += 1;
        }

        // 2. Direct diffused tracks (if any, e.g. soundtrack pieces)
        let top_diffused_tracks = top_k_from_scores(
            &ctx.state.graph,
            ctx.p,
            ctx.exclude_ids,
            "track:",
            Some(track_quota * 2),
        );

        for item in top_diffused_tracks {
            if item.score < ctx.min_rec_score {
                continue;
            }
            if track_recs_added >= track_quota {
                break;
            }
            let Some(("track", tid)) = parse_node_key(&item.node_key) else {
                continue;
            };

            let Ok((ttitle, aname, aid, bpm, pop)) = ctx.conn.query_row(
                "SELECT t.title, a.name, a.id, t.tempo, t.popularity FROM music_tracks t JOIN music_artists a ON a.id = t.artist_id WHERE t.id = ?",
                params![tid],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<f64>>(3)?,
                        r.get::<_, Option<i32>>(4)?,
                    ))
                },
            ) else {
                continue;
            };

            let norm_t = norm_track_title(&ttitle);
            if norm_t.is_empty()
                || !seen_rec_track_titles.insert(norm_t)
                || !seen_rec_track_ids.insert(tid)
                || ctx.seed_set.contains(&item.node_key)
                || !seen_rec_track_artists.insert(aid)
            {
                continue;
            }
            let bpm_str = bpm
                .filter(|&b| b > 0.0)
                .map(|b| format!(" • {} BPM", b.round() as i32))
                .unwrap_or_default();
            let trk_subtitle = if ctx.is_pt {
                format!("Música de {}{}", aname, bpm_str)
            } else {
                format!("Track by {}{}", aname, bpm_str)
            };
            let trk_reason = if ctx.is_pt {
                "Alta ressonância na difusão da trilha sonora e estilo"
            } else {
                "High resonance in diffusion path and style"
            };

            results.push(json!({
                "id": item.node_key,
                "title": ttitle,
                "type": "track",
                "subtitle": trk_subtitle,
                "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                "top_track": {
                    "title": ttitle,
                    "popularity": pop.unwrap_or(65),
                },
                "reason": trk_reason,
            }));

            let art_key = format!("artist:{}", aid);
            if !graph_nodes.iter().any(|n| n.id == art_key) {
                graph_nodes.push(GraphNodePayload {
                    id: art_key.clone(),
                    label: aname.clone(),
                    node_type: "artist".to_string(),
                    val: 11.5,
                    tags: Vec::new(),
                    desc: String::new(),
                    metadata: None,
                });
            }
            graph_nodes.push(GraphNodePayload {
                id: item.node_key.clone(),
                label: ttitle.clone(),
                node_type: "track".to_string(),
                val: 10.5,
                tags: Vec::new(),
                desc: String::new(),
                metadata: None,
            });
            graph_links.push(GraphLinkPayload {
                source: item.node_key.clone(),
                target: art_key,
                label: "performed_by".to_string(),
                weight: 2.5,
            });
            track_recs_added += 1;
        }

        // 3. Essential tracks from the balanced recommended artists
        if track_recs_added < track_quota && !recommended_artist_tracks.is_empty() {
            let mut tracks_to_add: Vec<TrackRecItem> = Vec::new();

            if seed_artist_keys.len() > 1 {
                let per_seed_track_quota =
                    ((track_quota - track_recs_added) / seed_artist_keys.len()).max(1);

                // Pass A: Balanced per-seed track extraction
                for s_key in &seed_artist_keys {
                    let mut seed_tracks: Vec<_> = recommended_artist_tracks
                        .iter()
                        .filter(|t| t.primary_seed.as_ref() == Some(s_key))
                        .collect();
                    seed_tracks.sort_by(|a, b| {
                        b.score
                            .partial_cmp(&a.score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });

                    for t in seed_tracks.into_iter().take(per_seed_track_quota) {
                        if tracks_to_add.len() + track_recs_added >= track_quota {
                            break;
                        }
                        if !tracks_to_add
                            .iter()
                            .any(|existing| existing.track.track_id == t.track.track_id)
                        {
                            tracks_to_add.push(t.clone());
                        }
                    }
                }

                // Pass B: Fill remaining from any recommended artist tracks
                for t in &recommended_artist_tracks {
                    if tracks_to_add.len() + track_recs_added >= track_quota {
                        break;
                    }
                    if !tracks_to_add
                        .iter()
                        .any(|existing| existing.track.track_id == t.track.track_id)
                    {
                        tracks_to_add.push(t.clone());
                    }
                }
            } else {
                for t in &recommended_artist_tracks {
                    if tracks_to_add.len() + track_recs_added >= track_quota {
                        break;
                    }
                    tracks_to_add.push(t.clone());
                }
            }

            for item in tracks_to_add {
                if track_recs_added >= track_quota {
                    break;
                }
                let t_key = format!("track:{}", item.track.track_id);
                let norm_t = norm_track_title(&item.track.title);
                if norm_t.is_empty()
                    || !seen_rec_track_titles.insert(norm_t)
                    || ctx.seed_set.contains(&t_key)
                    || !seen_rec_track_ids.insert(item.track.track_id)
                    || !seen_rec_track_artists.insert(item.artist_id)
                {
                    continue;
                }

                let bpm_str = item
                    .track
                    .tempo
                    .filter(|&b| b > 0.0)
                    .map(|b| format!(" • {} BPM", b.round() as i32))
                    .unwrap_or_default();
                let alb_str = item
                    .track
                    .album_title
                    .as_deref()
                    .map(|a| format!(" • Álbum: {}", a))
                    .unwrap_or_default();
                let trk_subtitle = if ctx.is_pt {
                    format!("Música de {}{}{}", item.artist_name, alb_str, bpm_str)
                } else {
                    format!("Track by {}{}{}", item.artist_name, alb_str, bpm_str)
                };
                let trk_reason = if ctx.is_pt {
                    format!("Faixa essencial de {} • {}", item.artist_name, item.reason)
                } else {
                    format!("Essential track by {} • {}", item.artist_name, item.reason)
                };

                results.push(json!({
                    "id": t_key,
                    "title": item.track.title,
                    "type": "track",
                    "subtitle": trk_subtitle,
                    "score": ((item.score as f64) * 950.0 * 100.0).round() / 100.0,
                    "top_track": {
                        "title": item.track.title,
                        "popularity": item.track.popularity.unwrap_or(65),
                    },
                    "reason": trk_reason,
                }));

                let art_key = format!("artist:{}", item.artist_id);
                if !graph_nodes.iter().any(|n| n.id == art_key) {
                    graph_nodes.push(GraphNodePayload {
                        id: art_key.clone(),
                        label: item.artist_name.clone(),
                        node_type: "artist".to_string(),
                        val: 11.5,
                        tags: Vec::new(),
                        desc: String::new(),
                        metadata: None,
                    });
                }
                graph_nodes.push(GraphNodePayload {
                    id: t_key.clone(),
                    label: item.track.title,
                    node_type: "track".to_string(),
                    val: 10.5,
                    tags: Vec::new(),
                    desc: String::new(),
                    metadata: None,
                });
                graph_links.push(GraphLinkPayload {
                    source: t_key,
                    target: art_key,
                    label: "performed_by".to_string(),
                    weight: 2.5,
                });
                track_recs_added += 1;
            }
        }
    }
}
