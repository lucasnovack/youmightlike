//! Music domain recommendation extractor (artists and tracks)
//! with multi-seed fairness, hub-degree normalization, and confluence prioritization.

use super::super::context::{is_steam_noise_tag, RecContext};
use crate::models::{
    is_tag_relation, lookup_artist_meta, lookup_top_track_full, parse_node_key, GraphLinkPayload,
    GraphNodePayload, TopTrackRow,
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
    top_track: Option<TopTrackRow>,
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

    let mut recommended_artist_tracks: Vec<(i64, String, f32, TopTrackRow, String, Option<String>)> =
        Vec::new();

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
    let has_music_seeds = !seed_artist_keys.is_empty()
        || ctx.active_seed_set.iter().any(|s| s.starts_with("track:"));

    // Pre-compute direct similar peer map and seed degree for all active music seeds
    let mut seed_peer_maps: HashMap<String, HashSet<String>> = HashMap::new();
    let mut seed_degrees: HashMap<String, usize> = HashMap::new();
    for s_key in &seed_artist_keys {
        let edges = ctx.state.graph.out_edges_by_key(s_key);
        seed_degrees.insert(s_key.clone(), edges.len());
        let mut peers = HashSet::new();
        for e in edges {
            if e.relation_type == "similar_to" {
                peers.insert(e.target_key.to_string());
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

    let top_diffused_artists =
        top_k_from_scores(&ctx.state.graph, ctx.p, ctx.exclude_ids, "artist:", Some(300));

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
        let aname_norm = aname
            .trim()
            .to_lowercase()
            .replace('’', "'")
            .replace(['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}'], "-");
        if seen_artist_names.contains(&aname_norm) {
            continue;
        }

        let out_edges = ctx.state.graph.out_edges_by_key(nkey);
        let mut connected_seeds: Vec<String> = Vec::new();
        let mut shared_tags: Vec<String> = Vec::new();
        let mut best_artist_bridge: Option<(String, String, String, f32)> = None;
        let mut matched_seed_game: Option<i64> = None;

        for e in &out_edges {
            if (e.relation_type == "soundtrack_composer" || e.relation_type == "soundtrack_for")
                && ctx.seed_games.contains(&parse_node_key(e.target_key).map(|(_, id)| id).unwrap_or(0))
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
            } else if e.relation_type == "similar_to" && ctx.active_seed_set.contains(e.target_key) {
                if !connected_seeds.contains(&e.target_key.to_string()) {
                    connected_seeds.push(e.target_key.to_string());
                }
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
            if peers.contains(nkey) && !connected_seeds.contains(s_key) {
                connected_seeds.push(s_key.clone());
                if best_artist_bridge.is_none() {
                    let s_name = ctx
                        .seed_artist_names
                        .get(s_key)
                        .cloned()
                        .unwrap_or_else(|| s_key.clone());
                    best_artist_bridge = Some((
                        s_key.clone(),
                        s_name,
                        "artist".to_string(),
                        1.5,
                    ));
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
                format!("Ponte de afinidade entre {}", names.join(" e "))
            } else {
                format!("Affinity bridge between {}", names.join(" and "))
            });
        } else if connected_seeds.len() == 1 {
            let s_name = ctx
                .seed_artist_names
                .get(&connected_seeds[0])
                .cloned()
                .unwrap_or_else(|| connected_seeds[0].clone());
            explanation_parts.push(if ctx.is_pt {
                format!("Similaridade direta com {}", s_name)
            } else {
                format!("Direct similarity with {}", s_name)
            });
        }

        if !shared_tags.is_empty() {
            let tags_joined = shared_tags.join(", ");
            explanation_parts.push(if ctx.is_pt {
                format!("Tags compartilhadas: {}", tags_joined)
            } else {
                format!("Shared tags: {}", tags_joined)
            });
        }

        let default_reason = if ctx.is_pt {
            "Afinidade e proximidade no grafo musical"
        } else {
            "Graph diffusion affinity and proximity"
        };
        let full_reason = if explanation_parts.is_empty() {
            default_reason.to_string()
        } else {
            explanation_parts.join(" • ")
        };

        // Score weighting with multi-seed confluence boost and hub degree normalization
        let mut boost = 1.0f32;
        if connected_seeds.len() >= 2 {
            // Highly prized confluence node bridging multiple seeds
            boost *= 2.5;
        } else if connected_seeds.len() == 1 {
            let s_key = &connected_seeds[0];
            let deg = seed_degrees.get(s_key).copied().unwrap_or(300);
            // Counteract hub diffusion penalty so mega-hubs (e.g. Kanye degree 1400)
            // don't have their neighbors starved by smaller niche seeds (degree 300)
            let deg_factor = ((deg as f32) / 300.0).sqrt().clamp(1.0, 2.3);
            boost *= deg_factor;
        }

        let top_track = lookup_top_track_full(ctx.conn, nid);
        let pop = top_track.as_ref().and_then(|t| t.popularity).unwrap_or(45) as f32;
        let pop_factor = if pop < 30.0 {
            0.35
        } else if pop < 50.0 {
            0.75
        } else {
            1.0 + (pop - 50.0) / 100.0 * 0.5
        };
        boost *= pop_factor;

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
            top_track,
        });
    }

    // -----------------------------------------------------------------------
    // Balanced Multi-Seed Artist Selection
    // -----------------------------------------------------------------------
    let mut chosen_candidates: Vec<CandidateArtist> = Vec::new();
    let mut chosen_keys: HashSet<String> = HashSet::new();

    if seed_artist_keys.len() > 1 && artist_quota > 0 {
        let per_seed_quota = (artist_quota / seed_artist_keys.len()).max(1);

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

        // Pass 2: Fair per-seed allocation (ensures each seed gets its own top artists)
        for s_key in &seed_artist_keys {
            let mut seed_cands: Vec<&CandidateArtist> = candidate_pool
                .iter()
                .filter(|c| {
                    (c.primary_seed.as_ref() == Some(s_key) || c.connected_seeds.contains(s_key))
                        && !chosen_keys.contains(&c.nkey)
                })
                .collect();
            seed_cands.sort_by(|a, b| {
                b.boosted_score
                    .partial_cmp(&a.boosted_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            for c in seed_cands.into_iter().take(per_seed_quota) {
                if chosen_candidates.len() >= artist_quota {
                    break;
                }
                if chosen_keys.insert(c.nkey.clone()) {
                    chosen_candidates.push(c.clone());
                }
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

        let top_track_val = c.top_track.as_ref().map(|t| {
            json!({
                "title": t.title,
                "popularity": t.popularity.unwrap_or(50),
            })
        });

        results.push(json!({
            "id": c.nkey,
            "title": c.aname,
            "type": "artist",
            "subtitle": sub,
            "score": ((c.boosted_score as f64) * 1000.0 * 100.0).round() / 100.0,
            "top_track": top_track_val,
            "reason": c.full_reason,
        }));

        graph_nodes.push(GraphNodePayload {
            id: c.nkey.clone(),
            label: c.aname.clone(),
            node_type: "artist".to_string(),
            val: 12.0,
            tags: Vec::new(),
            desc: String::new(),
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

        if let Some(ref trk) = c.top_track {
            let primary = c.primary_seed.clone().or_else(|| c.connected_seeds.first().cloned());
            recommended_artist_tracks.push((
                c.nid,
                c.aname.clone(),
                c.boosted_score,
                trk.clone(),
                c.full_reason.clone(),
                primary,
            ));
        }
    }

    // -----------------------------------------------------------------------
    // Balanced Multi-Seed Track Selection
    // -----------------------------------------------------------------------
    if track_quota > 0 {
        let mut track_recs_added = 0usize;
        let mut seen_rec_track_ids: HashSet<i64> = HashSet::new();
        let mut seen_rec_track_artists: HashSet<i64> = HashSet::new();

        // 1. Direct diffused tracks (if any, e.g. soundtrack pieces)
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

            if seen_rec_track_ids.insert(tid) && seen_rec_track_artists.insert(aid) {
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
                    });
                }
                graph_nodes.push(GraphNodePayload {
                    id: item.node_key.clone(),
                    label: ttitle.clone(),
                    node_type: "track".to_string(),
                    val: 10.5,
                    tags: Vec::new(),
                    desc: String::new(),
                });
                graph_links.push(GraphLinkPayload {
                    source: item.node_key.clone(),
                    target: art_key,
                    label: "performed_by".to_string(),
                    weight: 2.5,
                });
                track_recs_added += 1;
            }
        }

        // 2. Essential tracks from the balanced recommended artists
        if track_recs_added < track_quota && !recommended_artist_tracks.is_empty() {
            let mut tracks_to_add: Vec<(i64, String, f32, TopTrackRow, String)> = Vec::new();

            if seed_artist_keys.len() > 1 {
                let per_seed_track_quota = ((track_quota - track_recs_added) / seed_artist_keys.len()).max(1);

                // Pass A: Balanced per-seed track extraction
                for s_key in &seed_artist_keys {
                    let mut seed_tracks: Vec<_> = recommended_artist_tracks
                        .iter()
                        .filter(|t| t.5.as_ref() == Some(s_key))
                        .collect();
                    seed_tracks.sort_by(|a, b| {
                        b.2.partial_cmp(&a.2)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });

                    for t in seed_tracks.into_iter().take(per_seed_track_quota) {
                        if tracks_to_add.len() + track_recs_added >= track_quota {
                            break;
                        }
                        if !tracks_to_add.iter().any(|existing| existing.3.track_id == t.3.track_id) {
                            tracks_to_add.push((t.0, t.1.clone(), t.2, t.3.clone(), t.4.clone()));
                        }
                    }
                }

                // Pass B: Fill remaining from any recommended artist tracks
                for t in &recommended_artist_tracks {
                    if tracks_to_add.len() + track_recs_added >= track_quota {
                        break;
                    }
                    if !tracks_to_add.iter().any(|existing| existing.3.track_id == t.3.track_id) {
                        tracks_to_add.push((t.0, t.1.clone(), t.2, t.3.clone(), t.4.clone()));
                    }
                }
            } else {
                for t in &recommended_artist_tracks {
                    if tracks_to_add.len() + track_recs_added >= track_quota {
                        break;
                    }
                    tracks_to_add.push((t.0, t.1.clone(), t.2, t.3.clone(), t.4.clone()));
                }
            }

            for (a_id, a_name, a_score, trk, art_reason) in tracks_to_add {
                if track_recs_added >= track_quota {
                    break;
                }
                let t_key = format!("track:{}", trk.track_id);
                if ctx.seed_set.contains(&t_key)
                    || !seen_rec_track_ids.insert(trk.track_id)
                    || !seen_rec_track_artists.insert(a_id)
                {
                    continue;
                }

                let bpm_str = trk
                    .tempo
                    .filter(|&b| b > 0.0)
                    .map(|b| format!(" • {} BPM", b.round() as i32))
                    .unwrap_or_default();
                let trk_subtitle = if ctx.is_pt {
                    format!("Música de {}{}", a_name, bpm_str)
                } else {
                    format!("Track by {}{}", a_name, bpm_str)
                };
                let trk_reason = if ctx.is_pt {
                    format!("Faixa essencial de {} ({})", a_name, art_reason)
                } else {
                    format!("Essential track by {} ({})", a_name, art_reason)
                };

                results.push(json!({
                    "id": t_key,
                    "title": trk.title,
                    "type": "track",
                    "subtitle": trk_subtitle,
                    "score": ((a_score as f64) * 950.0 * 100.0).round() / 100.0,
                    "top_track": {
                        "title": trk.title,
                        "popularity": trk.popularity.unwrap_or(65),
                    },
                    "reason": trk_reason,
                }));

                let art_key = format!("artist:{}", a_id);
                if !graph_nodes.iter().any(|n| n.id == art_key) {
                    graph_nodes.push(GraphNodePayload {
                        id: art_key.clone(),
                        label: a_name.clone(),
                        node_type: "artist".to_string(),
                        val: 11.5,
                        tags: Vec::new(),
                        desc: String::new(),
                    });
                }
                graph_nodes.push(GraphNodePayload {
                    id: t_key.clone(),
                    label: trk.title,
                    node_type: "track".to_string(),
                    val: 10.5,
                    tags: Vec::new(),
                    desc: String::new(),
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
