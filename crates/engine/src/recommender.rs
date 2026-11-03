use crate::models::{
    collect_node_tags, is_soundtrack_relation, is_tag_relation, lookup_artist_meta,
    lookup_top_track_full, parse_node_key, AppState, EntityItem, GraphLinkPayload,
    GraphNodePayload, GraphPayload, RecommendationRequest, RecommendationResponse, TopTrackRow,
    TrackRecommendRequest, TrackRecommendationItem,
};
use crate::ppr::{
    run_personalized_pagerank_filtered, run_personalized_pagerank_scores, top_k_from_scores,
    ScoredNode,
};
use anyhow::Result;
use axum::{extract::State, http::StatusCode, Json};
use duckdb::params;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

pub fn compute_recommendations_sync(
    state: &AppState,
    req: &RecommendationRequest,
) -> Result<RecommendationResponse> {
    if req.seeds.is_empty() {
        return Ok(RecommendationResponse {
            seeds: Vec::new(),
            results: Vec::new(),
            graph: GraphPayload {
                nodes: Vec::new(),
                links: Vec::new(),
            },
        });
    }

    let mut sorted_seeds = req.seeds.clone();
    sorted_seeds.sort();
    let cache_key = format!(
        "{}:{}:{}:{}",
        sorted_seeds.join(","),
        req.target_type.as_deref().unwrap_or("all"),
        req.limit,
        req.lang
    );

    if let Ok(cache) = state.cache.lock() {
        if let Some(cached) = cache.get(&cache_key) {
            return Ok(cached);
        }
    }

    let seed_set: HashSet<String> = req.seeds.iter().cloned().collect();
    let mut seeds_resolved: Vec<EntityItem> = Vec::new();
    let mut diffusion_seeds: Vec<String> = Vec::new();
    let mut seed_artist_names: HashMap<String, String> = HashMap::new();
    let mut track_seed_artists: HashMap<String, (i64, String)> = HashMap::new();

    let conn = state.open_read_conn()?;

    // 1. Resolve seed node metadata
        for s in &req.seeds {
            let Some((prefix, nid)) = parse_node_key(s) else {
                continue;
            };
            match prefix {
                "artist" => {
                    if let Some((name, country)) = lookup_artist_meta(&conn, nid) {
                        seed_artist_names.insert(s.clone(), name.clone());
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name,
                            entity_type: "artist".to_string(),
                            subtitle: country,
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "movie" => {
                    if let Some(m) = state.catalog.movies.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: m.title.clone(),
                            entity_type: "movie".to_string(),
                            subtitle: m.release_year.map(|y| y.to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "series" => {
                    if let Some(s_meta) = state.catalog.series.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: s_meta.title.clone(),
                            entity_type: "series".to_string(),
                            subtitle: s_meta.start_year.map(|y| y.to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "game" => {
                    if let Some(g) = state.catalog.games.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: g.title.clone(),
                            entity_type: "game".to_string(),
                            subtitle: g.release_year.map(|y| y.to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "book" => {
                    if let Some(b) = state.catalog.books.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: b.title.clone(),
                            entity_type: "book".to_string(),
                            subtitle: b.first_publish_year.map(|y| y.to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "track" => {
                    if let Ok((t_title, a_name, a_id)) = conn.query_row(
                        "SELECT t.title, ma.name, t.artist_id FROM music_tracks t JOIN music_artists ma ON ma.id = t.artist_id WHERE t.id = ?",
                        params![nid],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
                    ) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: t_title,
                            entity_type: "track".to_string(),
                            subtitle: Some(a_name.clone()),
                            extra: None,
                        });
                        track_seed_artists.insert(s.clone(), (a_id, a_name.clone()));
                        let art_node = format!("artist:{}", a_id);
                        seed_artist_names.insert(art_node.clone(), a_name);
                        if !diffusion_seeds.contains(&art_node) {
                            diffusion_seeds.push(art_node);
                        }
                        if state.graph.get_node_id(s).is_some() && !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "keyword" => {
                    if let Some(kname) = state.catalog.keywords.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: format!("#{}", kname),
                            entity_type: "keyword".to_string(),
                            subtitle: Some("Tag".to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "person" => {
                    if let Some(pname) = state.catalog.people.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: pname.clone(),
                            entity_type: "person".to_string(),
                            subtitle: Some("Person".to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "developer" => {
                    if let Some(dname) = state.catalog.developers.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: dname.clone(),
                            entity_type: "developer".to_string(),
                            subtitle: Some("Studio".to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "author" => {
                    if let Some(aname) = state.catalog.authors.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: aname.clone(),
                            entity_type: "author".to_string(),
                            subtitle: Some("Author".to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                _ => {}
            }
        }

    // 2. Run PPR in-memory diffusion and obtain full stationary probability vector `p`
    let active_seeds: Vec<&str> = if diffusion_seeds.is_empty() {
        req.seeds.iter().map(|s| s.as_str()).collect()
    } else {
        diffusion_seeds.iter().map(|s| s.as_str()).collect()
    };

    let p = run_personalized_pagerank_scores(&state.graph, &active_seeds, 0.85, 25, 1e-6)
        .unwrap_or_else(|_| vec![0.0; state.graph.num_nodes]);

    let mut exclude_ids: HashSet<u32> = HashSet::new();
    for s in &req.seeds {
        if let Some(uid) = state.graph.get_node_id(s) {
            exclude_ids.insert(uid);
        }
    }
    for &s in &active_seeds {
        if let Some(uid) = state.graph.get_node_id(s) {
            exclude_ids.insert(uid);
        }
    }

    // Top diffused artists
    let top_diffused_artists =
        top_k_from_scores(&state.graph, &p, &exclude_ids, "artist:", Some(20));

    let active_seed_set: HashSet<String> = active_seeds.iter().map(|s| s.to_string()).collect();
    let mut affinitive_artist_keys: HashSet<String> = active_seed_set
        .iter()
        .filter(|s| s.starts_with("artist:"))
        .cloned()
        .collect();
    for a in top_diffused_artists.iter().take(12) {
        affinitive_artist_keys.insert(a.node_key.clone());
    }

    // Pre-compute seed neighbor maps for instant in-memory path explanations
    let mut seed_tag_counts: HashMap<i64, usize> = HashMap::new();
    let mut seed_people: HashSet<i64> = HashSet::new();
    let mut seed_devs: HashSet<i64> = HashSet::new();
    let mut seed_authors: HashSet<i64> = HashSet::new();
    let mut seed_artists: HashSet<i64> = HashSet::new();
    let mut seed_games: HashSet<i64> = HashSet::new();

    for &seed_key in &active_seeds {
        if let Some((pfx, nid)) = parse_node_key(seed_key) {
            if pfx == "artist" {
                seed_artists.insert(nid);
            } else if pfx == "game" {
                seed_games.insert(nid);
            }
        }
        let mut seen_seed_tags = HashSet::new();
        for edge in state.graph.out_edges_by_key(seed_key) {
            if let Some((tpfx, tid)) = parse_node_key(edge.target_key) {
                if tpfx == "keyword" && is_tag_relation(edge.relation_type) {
                    if seen_seed_tags.insert(tid) {
                        *seed_tag_counts.entry(tid).or_insert(0) += 1;
                    }
                } else if tpfx == "person"
                    && matches!(edge.relation_type, "has_director" | "has_composer" | "has_writer")
                {
                    seed_people.insert(tid);
                } else if tpfx == "developer" && edge.relation_type == "developed_by" {
                    seed_devs.insert(tid);
                } else if tpfx == "author" && edge.relation_type == "written_by" {
                    seed_authors.insert(tid);
                } else if tpfx == "artist" && matches!(edge.relation_type, "soundtrack_composer" | "soundtrack_featured" | "is_artist") {
                    seed_artists.insert(tid);
                } else if tpfx == "game" && edge.relation_type == "soundtrack_for" {
                    seed_games.insert(tid);
                }
            }
        }
    }

    if seed_tag_counts.len() < 5 {
        for a in top_diffused_artists.iter().take(5) {
            for edge in state.graph.out_edges_by_key(&a.node_key) {
                if let Some(("keyword", tid)) = parse_node_key(edge.target_key) {
                    if is_tag_relation(edge.relation_type) {
                        *seed_tag_counts.entry(tid).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let mut results: Vec<Value> = Vec::new();
    let mut graph_nodes: Vec<GraphNodePayload> = Vec::new();
    let mut graph_links: Vec<GraphLinkPayload> = Vec::new();

    let steam_noise_tags: HashSet<&str> = [
        "adjustable difficulty", "adjustable text size", "camera comfort", "captions available",
        "chat speech-to-text", "chat text-to-speech", "color alternatives", "commentary available",
        "cross-platform multiplayer", "custom volume controls", "family sharing", "full controller support",
        "hdr available", "in-app purchases", "includes source sdk", "includes level editor",
        "keyboard only option", "lan co-op", "lan pvp", "mods", "mods (require hl2)", "mouse only option",
        "narrated game menus", "partial controller support", "playable without timed input",
        "remote play together", "remote play on phone", "remote play on tv", "remote play on tablet",
        "save anytime", "shared/split screen", "shared/split screen co-op", "shared/split screen pvp",
        "single-player", "multi-player", "multiplayer", "stats", "steam achievements", "steam cloud",
        "steam leaderboards", "steam timeline", "steam trading cards", "steam turn notifications",
        "steam workshop", "steamvr collectibles", "stereo sound", "subtitle options", "surround sound",
        "touch only option", "tracked controller support", "vr only", "vr support", "vr supported",
        "valve anti-cheat enabled",
    ]
    .into_iter()
    .collect();

    // 3. Add seed nodes and structural anchors to subgraph
    for seed_item in &seeds_resolved {
        graph_nodes.push(GraphNodePayload {
            id: seed_item.id.clone(),
            label: seed_item.name.clone(),
            node_type: "seed".to_string(),
            val: 18.0,
            tags: Vec::new(),
            desc: String::new(),
        });

        if seed_item.entity_type == "track" {
            if let Some((a_id, a_name)) = track_seed_artists.get(&seed_item.id) {
                let art_key = format!("artist:{}", a_id);
                if !graph_nodes.iter().any(|n| n.id == art_key) {
                    graph_nodes.push(GraphNodePayload {
                        id: art_key.clone(),
                        label: a_name.clone(),
                        node_type: "artist".to_string(),
                        val: 14.0,
                        tags: Vec::new(),
                        desc: String::new(),
                    });
                }
                graph_links.push(GraphLinkPayload {
                    source: seed_item.id.clone(),
                    target: art_key,
                    label: "performed_by".to_string(),
                    weight: 2.5,
                });
            }
        }

        // Structural anchors for each seed (filtering infrastructure noise)
        let mut anchor_edges = state.graph.out_edges_by_key(&seed_item.id);
        anchor_edges.retain(|e| {
            if let Some(("keyword", tid)) = parse_node_key(e.target_key) {
                if let Some(kname) = state.catalog.keywords.get(&tid) {
                    return !steam_noise_tags.contains(kname.as_str());
                }
            }
            true
        });
        anchor_edges.sort_by(|a, b| {
            let prio = |rel: &str| match rel {
                "developed_by" => 1,
                "has_director" => 2,
                "has_writer" => 3,
                "has_tag" => 4,
                _ => 5,
            };
            prio(a.relation_type)
                .cmp(&prio(b.relation_type))
                .then_with(|| {
                    b.base_weight
                        .partial_cmp(&a.base_weight)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        for edge in anchor_edges.into_iter().take(2) {
            if let Some((t_pfx, t_nid)) = parse_node_key(edge.target_key) {
                let t_label = match t_pfx {
                    "developer" => state.catalog.developers.get(&t_nid).cloned(),
                    "keyword" => state
                        .catalog
                        .keywords
                        .get(&t_nid)
                        .map(|k| format!("#{}", k)),
                    "person" => state.catalog.people.get(&t_nid).cloned(),
                    "author" => state.catalog.authors.get(&t_nid).cloned(),
                    _ => None,
                };
                if let Some(label) = t_label {
                    if !graph_nodes.iter().any(|n| n.id == edge.target_key) {
                        graph_nodes.push(GraphNodePayload {
                            id: edge.target_key.to_string(),
                            label,
                            node_type: t_pfx.to_string(),
                            val: 9.0,
                            tags: Vec::new(),
                            desc: String::new(),
                        });
                        graph_links.push(GraphLinkPayload {
                            source: seed_item.id.clone(),
                            target: edge.target_key.to_string(),
                            label: edge.relation_type.to_string(),
                            weight: edge.base_weight,
                        });
                    }
                }
            }
        }
    }

    if diffusion_seeds.len() > 1 {
        let mut common_tags: Vec<(i64, usize)> = seed_tag_counts
            .iter()
            .filter(|&(kid, &cnt)| {
                if cnt < 2 {
                    return false;
                }
                if let Some(kname) = state.catalog.keywords.get(kid) {
                    !steam_noise_tags.contains(kname.as_str())
                } else {
                    false
                }
            })
            .map(|(&kid, &cnt)| (kid, cnt))
            .collect();
        common_tags.sort_by(|a, b| b.1.cmp(&a.1));

        for (kid, _) in common_tags.into_iter().take(5) {
            if let Some(kname) = state.catalog.keywords.get(&kid) {
                let b_id = format!("keyword:{}", kid);
                if !graph_nodes.iter().any(|n| n.id == b_id) {
                    graph_nodes.push(GraphNodePayload {
                        id: b_id,
                        label: format!("#{}", kname),
                        node_type: "keyword".to_string(),
                        val: 9.0,
                        tags: Vec::new(),
                        desc: String::new(),
                    });
                }
            }
        }
    }

    // Domain-balanced quotas for true transmedia discovery
    let (artist_quota, track_quota, movie_quota, series_quota, game_quota, book_quota) =
        match req.target_type.as_deref() {
            Some("music") => {
                let a_q = (req.limit + 1) / 2;
                let t_q = req.limit.saturating_sub(a_q).max(1);
                (a_q, t_q, 0, 0, 0, 0)
            }
            Some("movie") => (0, 0, req.limit, 0, 0, 0),
            Some("series") => (0, 0, 0, req.limit, 0, 0),
            Some("game") => (0, 0, 0, 0, req.limit, 0),
            Some("book") => (0, 0, 0, 0, 0, req.limit),
            _ => {
                let a_q = (req.limit * 28 / 100).max(3);
                let t_q = (req.limit * 28 / 100).max(3);
                let m_q = (req.limit * 20 / 100).max(2);
                let s_q = (req.limit * 16 / 100).max(2);
                let g_q = req.limit.saturating_sub(a_q + t_q + m_q + s_q).max(1);
                (a_q, t_q, m_q, s_q, g_q, 1)
            }
        };

    let is_pt = req.lang == "pt";
    /// Minimum PPR diffusion score floor (relative scale) to avoid statistical noise
    /// and drop disconnected items from forced quotas with zero affinity.
    const MIN_REC_SCORE: f32 = 5e-5;

        // -------------------------------------------------------------------
        // A. RECOMMENDED ARTISTS (`artist:*`)
        // -------------------------------------------------------------------
        let mut artist_recs_added = 0usize;
        let mut recommended_artist_tracks: Vec<(i64, String, f32, TopTrackRow, String)> =
            Vec::new();

        if artist_quota > 0 || track_quota > 0 {
            for item in &top_diffused_artists {
                if item.score < MIN_REC_SCORE {
                    continue;
                }
                let nkey = &item.node_key;
                if seed_set.contains(nkey) {
                    continue;
                }
                let Some(("artist", nid)) = parse_node_key(nkey) else {
                    continue;
                };

                let Some((aname, country)) = lookup_artist_meta(&conn, nid) else {
                    continue;
                };

                // In-memory path explanation via directed CSR edges
                let mut explanation_parts: Vec<String> = Vec::new();
                let mut direct_co_listening = false;
                let mut shared_tags: Vec<String> = Vec::new();
                // Track connecting bridge to seeds or shared tags to avoid isolated nodes
                let mut best_artist_bridge: Option<(String, String, String, f32)> = None;

                for e in state.graph.out_edges_by_key(nkey) {
                    if (e.relation_type == "soundtrack_composer" || e.relation_type == "soundtrack_for")
                        && seed_games.contains(&parse_node_key(e.target_key).map(|(_, id)| id).unwrap_or(0))
                    {
                        if let Some(("game", gid)) = parse_node_key(e.target_key) {
                            if let Some(g) = state.catalog.games.get(&gid) {
                                explanation_parts.push(if is_pt {
                                    format!("Compositor da trilha de {}", g.title)
                                } else {
                                    format!("Soundtrack composer for {}", g.title)
                                });
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
                    } else if e.relation_type == "similar_to"
                        && active_seed_set.contains(e.target_key)
                        && !direct_co_listening
                    {
                        if let Some(("artist", s_aid)) = parse_node_key(e.target_key) {
                            let s_name = seed_artist_names
                                .get(e.target_key)
                                .cloned()
                                .or_else(|| lookup_artist_meta(&conn, s_aid).map(|(n, _)| n));
                            if let Some(sname) = s_name {
                                explanation_parts.push(if is_pt {
                                    format!("Similaridade direta com {}", sname)
                                } else {
                                    format!("Direct similarity with {}", sname)
                                });
                                direct_co_listening = true;
                                if best_artist_bridge.is_none() {
                                    best_artist_bridge = Some((
                                        e.target_key.to_string(),
                                        sname,
                                        "artist".to_string(),
                                        e.base_weight,
                                    ));
                                }
                            }
                        }
                    } else if is_tag_relation(e.relation_type) && e.target_key.starts_with("keyword:")
                    {
                        if let Some(("keyword", kid)) = parse_node_key(e.target_key) {
                            if seed_tag_counts.contains_key(&kid) && shared_tags.len() < 3 {
                                if let Some(kname) = state.catalog.keywords.get(&kid) {
                                    let tfmt = format!("#{}", kname);
                                    if !shared_tags.contains(&tfmt) {
                                        shared_tags.push(tfmt);
                                    }
                                    if best_artist_bridge.is_none() && !steam_noise_tags.contains(kname.as_str()) {
                                        best_artist_bridge = Some((
                                            e.target_key.to_string(),
                                            format!("#{}", kname),
                                            "keyword".to_string(),
                                            e.base_weight,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }

                if !shared_tags.is_empty() {
                    let tags_joined = shared_tags.join(", ");
                    explanation_parts.push(if is_pt {
                        format!("Tags compartilhadas: {}", tags_joined)
                    } else {
                        format!("Shared tags: {}", tags_joined)
                    });
                }

                let default_reason = if is_pt {
                    "Afinidade e proximidade no grafo musical"
                } else {
                    "Graph diffusion affinity and proximity"
                };
                let full_reason = if explanation_parts.is_empty() {
                    default_reason.to_string()
                } else {
                    explanation_parts.join(" • ")
                };

                let top_track_row = lookup_top_track_full(&conn, nid);
                let top_track_val = top_track_row.as_ref().map(|t| {
                    json!({
                        "title": t.title,
                        "popularity": t.popularity.unwrap_or(50),
                    })
                });

                if let Some(ref trk) = top_track_row {
                    recommended_artist_tracks.push((
                        nid,
                        aname.clone(),
                        item.score,
                        trk.clone(),
                        full_reason.clone(),
                    ));
                }

                if artist_recs_added < artist_quota {
                    let sub = country
                        .map(|c| {
                            if is_pt {
                                format!("Artista • {}", c)
                            } else {
                                format!("Artist • {}", c)
                            }
                        })
                        .unwrap_or_else(|| {
                            if is_pt {
                                "Artista".to_string()
                            } else {
                                "Artist".to_string()
                            }
                        });

                    results.push(json!({
                        "id": nkey,
                        "title": aname,
                        "type": "artist",
                        "subtitle": sub,
                        "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                        "top_track": top_track_val,
                        "reason": full_reason,
                    }));

                    graph_nodes.push(GraphNodePayload {
                        id: nkey.clone(),
                        label: aname.clone(),
                        node_type: "artist".to_string(),
                        val: 12.0,
                        tags: Vec::new(),
                        desc: String::new(),
                    });

                    // If artist has a connecting bridge, inject it into graph_nodes and graph_links
                    if let Some((b_key, b_label, b_type, b_weight)) = best_artist_bridge {
                        if !graph_nodes.iter().any(|n| n.id == b_key) {
                            graph_nodes.push(GraphNodePayload {
                                id: b_key.clone(),
                                label: b_label,
                                node_type: b_type,
                                val: 9.0,
                                tags: Vec::new(),
                                desc: String::new(),
                            });
                        }
                        graph_links.push(GraphLinkPayload {
                            source: nkey.clone(),
                            target: b_key,
                            label: if is_pt { "conectado_a".to_string() } else { "connected_to".to_string() },
                            weight: b_weight,
                        });
                    }

                    artist_recs_added += 1;
                }
            }
        }

        // -------------------------------------------------------------------
        // B. RECOMMENDED TRACKS (`track:*`)
        // -------------------------------------------------------------------
        if track_quota > 0 {
            let mut track_recs_added = 0usize;
            let mut seen_rec_track_ids: HashSet<i64> = HashSet::new();
            let mut seen_rec_track_artists: HashSet<i64> = HashSet::new();

            let top_diffused_tracks =
                top_k_from_scores(&state.graph, &p, &exclude_ids, "track:", Some(track_quota * 4));

            for item in top_diffused_tracks {
                if item.score < MIN_REC_SCORE {
                    continue;
                }
                if track_recs_added >= track_quota {
                    break;
                }
                let Some(("track", tid)) = parse_node_key(&item.node_key) else {
                    continue;
                };

                let Ok((ttitle, aname, aid, bpm, pop)) = conn.query_row(
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
                    let trk_subtitle = if is_pt {
                        format!("Música de {}{}", aname, bpm_str)
                    } else {
                        format!("Track by {}{}", aname, bpm_str)
                    };
                    let trk_reason = if is_pt {
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

            for (a_id, a_name, a_score, trk, art_reason) in recommended_artist_tracks {
                if track_recs_added >= track_quota {
                    break;
                }
                let t_key = format!("track:{}", trk.track_id);
                if seed_set.contains(&t_key)
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
                let trk_subtitle = if is_pt {
                    format!("Música de {}{}", a_name, bpm_str)
                } else {
                    format!("Track by {}{}", a_name, bpm_str)
                };
                let trk_reason = if is_pt {
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

        // -------------------------------------------------------------------
        // C. RECOMMENDED MOVIES (`movie:*`)
        // -------------------------------------------------------------------
        if movie_quota > 0 {
            let raw_movies = top_k_from_scores(&state.graph, &p, &exclude_ids, "movie:", Some(35));

            let mut ranked_movies: Vec<(ScoredNode, Vec<(String, String)>)> = Vec::new();
            for m_item in raw_movies {
                if m_item.score < MIN_REC_SCORE {
                    continue;
                }
                let mut ost_artists: Vec<(String, String)> = Vec::new();
                let mut seen_ost_names: HashSet<String> = HashSet::new();

                for e in state.graph.out_edges_by_key(&m_item.node_key) {
                    if is_soundtrack_relation(e.relation_type)
                        && e.target_key.starts_with("artist:")
                        && affinitive_artist_keys.contains(e.target_key)
                    {
                        if let Some(("artist", aid)) = parse_node_key(e.target_key) {
                            let aname = seed_artist_names
                                .get(e.target_key)
                                .cloned()
                                .or_else(|| lookup_artist_meta(&conn, aid).map(|(n, _)| n));
                            if let Some(name) = aname {
                                if seen_ost_names.insert(name.clone()) {
                                    ost_artists.push((e.target_key.to_string(), name));
                                }
                            }
                        }
                    }
                }

                let ost_boost = 1.0 + (ost_artists.len() as f32) * 0.65;
                let boosted = ScoredNode {
                    node_key: m_item.node_key,
                    score: m_item.score * ost_boost,
                };
                ranked_movies.push((boosted, ost_artists));
            }

            ranked_movies.sort_by(|a, b| {
                b.0.score
                    .partial_cmp(&a.0.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut movie_recs_added = 0usize;
            for (item, ost_artists) in ranked_movies {
                if movie_recs_added >= movie_quota {
                    break;
                }
                let nkey = &item.node_key;
                if seed_set.contains(nkey) {
                    continue;
                }
                let Some(("movie", nid)) = parse_node_key(nkey) else {
                    continue;
                };
                let Some(m) = state.catalog.movies.get(&nid) else {
                    continue;
                };

                let mut movie_explanation: Vec<String> = Vec::new();

                if !ost_artists.is_empty() {
                    let names: Vec<&str> = ost_artists
                        .iter()
                        .take(4)
                        .map(|(_, n)| n.as_str())
                        .collect();
                    movie_explanation.push(if is_pt {
                        format!("Trilha sonora com {}", names.join(", "))
                    } else {
                        format!("Soundtrack featuring {}", names.join(", "))
                    });

                    for (art_key, art_name) in ost_artists.iter().take(2) {
                        if !graph_nodes.iter().any(|n| &n.id == art_key) {
                            graph_nodes.push(GraphNodePayload {
                                id: art_key.clone(),
                                label: art_name.clone(),
                                node_type: "artist".to_string(),
                                val: 11.5,
                                tags: Vec::new(),
                                desc: String::new(),
                            });
                        }
                    }
                }

                let mut found_person: Option<String> = None;
                let mut shared_movie_tags: Vec<String> = Vec::new();

                for e in state.graph.out_edges_by_key(nkey) {
                    if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                        if tpfx == "person" && found_person.is_none() && seed_people.contains(&tid)
                        {
                            found_person = state.catalog.people.get(&tid).cloned();
                        } else if tpfx == "keyword"
                            && is_tag_relation(e.relation_type)
                            && seed_tag_counts.contains_key(&tid)
                            && shared_movie_tags.len() < 3
                        {
                            if let Some(kname) = state.catalog.keywords.get(&tid) {
                                let tfmt = format!("#{}", kname);
                                if !shared_movie_tags.contains(&tfmt) {
                                    shared_movie_tags.push(tfmt);
                                }
                            }
                        }
                    }
                }

                if let Some(pname) = found_person {
                    movie_explanation.push(if is_pt {
                        format!("Mesmo realizador/compositor ({})", pname)
                    } else {
                        format!("Same director/composer ({})", pname)
                    });
                }
                if !shared_movie_tags.is_empty() {
                    let tag_str = shared_movie_tags.join(", ");
                    movie_explanation.push(if is_pt {
                        format!("Tags em comum: {}", tag_str)
                    } else {
                        format!("Shared tags: {}", tag_str)
                    });
                }

                let default_movie_reason = if is_pt {
                    "Afinidade de difusão de gênero e estilo"
                } else {
                    "Genre and style diffusion affinity"
                };
                let full_movie_reason = if movie_explanation.is_empty() {
                    default_movie_reason.to_string()
                } else {
                    movie_explanation.join(" • ")
                };

                let year_str = m
                    .release_year
                    .map(|y| y.to_string())
                    .unwrap_or_else(|| "N/A".to_string());
                let vote_str = m
                    .vote_average
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_else(|| "N/A".to_string());
                let movie_subtitle = if is_pt {
                    format!("Filme ({}) • Nota {}", year_str, vote_str)
                } else {
                    format!("Movie ({}) • Rating: {}", year_str, vote_str)
                };

                results.push(json!({
                    "id": nkey,
                    "title": m.title,
                    "type": "movie",
                    "subtitle": movie_subtitle,
                    "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                    "reason": full_movie_reason,
                }));
                graph_nodes.push(GraphNodePayload {
                    id: nkey.clone(),
                    label: m.title.clone(),
                    node_type: "movie".to_string(),
                    val: 12.5,
                    tags: Vec::new(),
                    desc: String::new(),
                });
                movie_recs_added += 1;
            }
        }

        // -------------------------------------------------------------------
        // D. RECOMMENDED SERIES & MINI-SERIES (`series:*`)
        // -------------------------------------------------------------------
        if series_quota > 0 {
            let raw_series =
                top_k_from_scores(&state.graph, &p, &exclude_ids, "series:", Some(35));

            let mut ranked_series: Vec<(ScoredNode, Vec<(String, String)>)> = Vec::new();
            for s_item in raw_series {
                if s_item.score < MIN_REC_SCORE {
                    continue;
                }
                let mut ost_artists: Vec<(String, String)> = Vec::new();
                let mut seen_ost_names: HashSet<String> = HashSet::new();

                for e in state.graph.out_edges_by_key(&s_item.node_key) {
                    if is_soundtrack_relation(e.relation_type)
                        && e.target_key.starts_with("artist:")
                        && affinitive_artist_keys.contains(e.target_key)
                    {
                        if let Some(("artist", aid)) = parse_node_key(e.target_key) {
                            let aname = seed_artist_names
                                .get(e.target_key)
                                .cloned()
                                .or_else(|| lookup_artist_meta(&conn, aid).map(|(n, _)| n));
                            if let Some(name) = aname {
                                if seen_ost_names.insert(name.clone()) {
                                    ost_artists.push((e.target_key.to_string(), name));
                                }
                            }
                        }
                    }
                }

                let ost_boost = 1.0 + (ost_artists.len() as f32) * 0.65;
                let boosted = ScoredNode {
                    node_key: s_item.node_key,
                    score: s_item.score * ost_boost,
                };
                ranked_series.push((boosted, ost_artists));
            }

            ranked_series.sort_by(|a, b| {
                b.0.score
                    .partial_cmp(&a.0.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut series_recs_added = 0usize;
            for (item, ost_artists) in ranked_series {
                if series_recs_added >= series_quota {
                    break;
                }
                let nkey = &item.node_key;
                if seed_set.contains(nkey) {
                    continue;
                }
                let Some(("series", nid)) = parse_node_key(nkey) else {
                    continue;
                };
                let Some(s) = state.catalog.series.get(&nid) else {
                    continue;
                };

                let mut series_explanation: Vec<String> = Vec::new();

                if !ost_artists.is_empty() {
                    let names: Vec<&str> = ost_artists
                        .iter()
                        .take(4)
                        .map(|(_, n)| n.as_str())
                        .collect();
                    series_explanation.push(if is_pt {
                        format!("Trilha sonora com {}", names.join(", "))
                    } else {
                        format!("Soundtrack featuring {}", names.join(", "))
                    });

                    for (art_key, art_name) in ost_artists.iter().take(2) {
                        if !graph_nodes.iter().any(|n| &n.id == art_key) {
                            graph_nodes.push(GraphNodePayload {
                                id: art_key.clone(),
                                label: art_name.clone(),
                                node_type: "artist".to_string(),
                                val: 11.5,
                                tags: Vec::new(),
                                desc: String::new(),
                            });
                        }
                    }
                }

                let mut found_person: Option<String> = None;
                let mut shared_series_tags: Vec<String> = Vec::new();

                for e in state.graph.out_edges_by_key(nkey) {
                    if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                        if tpfx == "person" && found_person.is_none() && seed_people.contains(&tid)
                        {
                            found_person = state.catalog.people.get(&tid).cloned();
                        } else if tpfx == "keyword"
                            && is_tag_relation(e.relation_type)
                            && seed_tag_counts.contains_key(&tid)
                            && shared_series_tags.len() < 3
                        {
                            if let Some(kname) = state.catalog.keywords.get(&tid) {
                                let tfmt = format!("#{}", kname);
                                if !shared_series_tags.contains(&tfmt) {
                                    shared_series_tags.push(tfmt);
                                }
                            }
                        }
                    }
                }

                if let Some(pname) = found_person {
                    series_explanation.push(if is_pt {
                        format!("Mesmo diretor/roteirista ({})", pname)
                    } else {
                        format!("Same director/writer ({})", pname)
                    });
                }
                if !shared_series_tags.is_empty() {
                    let tag_str = shared_series_tags.join(", ");
                    series_explanation.push(if is_pt {
                        format!("Tags em comum: {}", tag_str)
                    } else {
                        format!("Shared tags: {}", tag_str)
                    });
                }

                let default_series_reason = if is_pt {
                    "Afinidade temática e difusão de estilo"
                } else {
                    "Thematic and style diffusion affinity"
                };
                let full_series_reason = if series_explanation.is_empty() {
                    default_series_reason.to_string()
                } else {
                    series_explanation.join(" • ")
                };

                let years_str = match (s.start_year, s.end_year) {
                    (Some(st), Some(e)) if st != e => format!("{}-{}", st, e),
                    (Some(st), _) => st.to_string(),
                    _ => "N/A".to_string(),
                };
                let vote_str = s
                    .vote_average
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_else(|| "N/A".to_string());
                let type_label = if s.series_type == "tvMiniSeries" {
                    if is_pt {
                        "Minissérie"
                    } else {
                        "Mini-Series"
                    }
                } else {
                    if is_pt {
                        "Série"
                    } else {
                        "Series"
                    }
                };
                let series_subtitle = if is_pt {
                    format!("{} ({}) • Nota {}", type_label, years_str, vote_str)
                } else {
                    format!("{} ({}) • Rating: {}", type_label, years_str, vote_str)
                };

                results.push(json!({
                    "id": nkey,
                    "title": s.title,
                    "type": "series",
                    "subtitle": series_subtitle,
                    "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                    "reason": full_series_reason,
                }));
                graph_nodes.push(GraphNodePayload {
                    id: nkey.clone(),
                    label: s.title.clone(),
                    node_type: "series".to_string(),
                    val: 12.5,
                    tags: Vec::new(),
                    desc: String::new(),
                });
                series_recs_added += 1;
            }
        }

        // -------------------------------------------------------------------
        // E. RECOMMENDED GAMES (`game:*`)
        // -------------------------------------------------------------------
        if game_quota > 0 {
            let top_games =
                top_k_from_scores(&state.graph, &p, &exclude_ids, "game:", Some(game_quota + 4));
            let mut game_recs_added = 0usize;

            for item in top_games {
                if item.score < MIN_REC_SCORE {
                    continue;
                }
                if game_recs_added >= game_quota {
                    break;
                }
                let nkey = &item.node_key;
                if seed_set.contains(nkey) {
                    continue;
                }
                let Some(("game", nid)) = parse_node_key(nkey) else {
                    continue;
                };
                let Some(g) = state.catalog.games.get(&nid) else {
                    continue;
                };

                let mut game_explanation: Vec<String> = Vec::new();
                let mut found_composer: Option<String> = None;
                let mut found_dev: Option<String> = None;
                let mut specific_tags: Vec<String> = Vec::new();
                let mut fallback_tags: Vec<String> = Vec::new();
                let mut best_game_bridge: Option<(String, String, String)> = None;

                for e in state.graph.out_edges_by_key(nkey) {
                    if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                        if tpfx == "artist" && (e.relation_type == "soundtrack_composer" || e.relation_type == "soundtrack_for") && seed_artists.contains(&tid) {
                            if found_composer.is_none() {
                                let cname = seed_artist_names.get(e.target_key)
                                    .cloned()
                                    .or_else(|| lookup_artist_meta(&conn, tid).map(|(n, _)| n));
                                if let Some(cname) = cname {
                                    best_game_bridge = Some((e.target_key.to_string(), cname.clone(), "artist".to_string()));
                                    found_composer = Some(cname);
                                }
                            }
                        } else if tpfx == "developer" && found_dev.is_none() && seed_devs.contains(&tid) {
                            found_dev = state.catalog.developers.get(&tid).cloned();
                            if let Some(ref dname) = found_dev {
                                best_game_bridge = Some((e.target_key.to_string(), dname.clone(), "developer".to_string()));
                            }
                        } else if tpfx == "keyword"
                            && is_tag_relation(e.relation_type)
                            && seed_tag_counts.contains_key(&tid)
                        {
                            if let Some(kname) = state.catalog.keywords.get(&tid) {
                                let tfmt = format!("#{}", kname);
                                if !steam_noise_tags.contains(kname.as_str()) {
                                    if specific_tags.len() < 3 && !specific_tags.contains(&tfmt) {
                                        specific_tags.push(tfmt);
                                    }
                                    if best_game_bridge.is_none() {
                                        best_game_bridge = Some((e.target_key.to_string(), format!("#{}", kname), "keyword".to_string()));
                                    }
                                } else if fallback_tags.len() < 2 && !fallback_tags.contains(&tfmt)
                                {
                                    fallback_tags.push(tfmt);
                                }
                            }
                        }
                    }
                }

                if let Some(cname) = found_composer {
                    game_explanation.push(if is_pt {
                        format!("Trilha sonora por {}", cname)
                    } else {
                        format!("Soundtrack by {}", cname)
                    });
                }
                if let Some(dname) = found_dev {
                    game_explanation.push(if is_pt {
                        format!("Mesmo estúdio ({})", dname)
                    } else {
                        format!("Same studio ({})", dname)
                    });
                }
                if !specific_tags.is_empty() {
                    let tag_str = specific_tags.join(", ");
                    game_explanation.push(if is_pt {
                        format!("Tags em comum: {}", tag_str)
                    } else {
                        format!("Shared tags: {}", tag_str)
                    });
                } else if !fallback_tags.is_empty() {
                    game_explanation.push(format!("Tags: {}", fallback_tags.join(", ")));
                }

                let default_game_reason = if is_pt {
                    "Afinidade de jogabilidade e universo temático"
                } else {
                    "Gameplay and thematic universe affinity"
                };
                let full_game_reason = if game_explanation.is_empty() {
                    default_game_reason.to_string()
                } else {
                    game_explanation.join(" • ")
                };

                let year_str = g
                    .release_year
                    .map(|y| y.to_string())
                    .unwrap_or_else(|| "N/A".to_string());
                let game_subtitle = if is_pt {
                    format!("Jogo ({})", year_str)
                } else {
                    format!("Game ({})", year_str)
                };

                results.push(json!({
                    "id": nkey,
                    "title": g.title,
                    "type": "game",
                    "subtitle": game_subtitle,
                    "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                    "reason": full_game_reason,
                }));
                graph_nodes.push(GraphNodePayload {
                    id: nkey.clone(),
                    label: g.title.clone(),
                    node_type: "game".to_string(),
                    val: 12.0,
                    tags: Vec::new(),
                    desc: String::new(),
                });
                if let Some((b_id, b_label, b_type)) = best_game_bridge {
                    if !graph_nodes.iter().any(|n| n.id == b_id) {
                        graph_nodes.push(GraphNodePayload {
                            id: b_id,
                            label: b_label,
                            node_type: b_type,
                            val: 9.0,
                            tags: Vec::new(),
                            desc: String::new(),
                        });
                    }
                }
                game_recs_added += 1;
            }
        }

        // -------------------------------------------------------------------
        // F. RECOMMENDED BOOKS (`book:*`)
        // -------------------------------------------------------------------
        if book_quota > 0 && results.len() < req.limit {
            let top_books =
                top_k_from_scores(&state.graph, &p, &exclude_ids, "book:", Some(book_quota + 2));
            let mut book_recs_added = 0usize;

            for item in top_books {
                if item.score < MIN_REC_SCORE {
                    continue;
                }
                if book_recs_added >= book_quota || results.len() >= req.limit {
                    break;
                }
                let nkey = &item.node_key;
                if seed_set.contains(nkey) {
                    continue;
                }
                let Some(("book", nid)) = parse_node_key(nkey) else {
                    continue;
                };
                let Some(b) = state.catalog.books.get(&nid) else {
                    continue;
                };

                let mut book_explanation: Vec<String> = Vec::new();
                let mut found_author: Option<String> = None;
                let mut shared_book_tags: Vec<String> = Vec::new();

                for e in state.graph.out_edges_by_key(nkey) {
                    if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                        if tpfx == "author" && found_author.is_none() && seed_authors.contains(&tid)
                        {
                            found_author = state.catalog.authors.get(&tid).cloned();
                        } else if tpfx == "keyword"
                            && is_tag_relation(e.relation_type)
                            && seed_tag_counts.contains_key(&tid)
                            && shared_book_tags.len() < 3
                        {
                            if let Some(kname) = state.catalog.keywords.get(&tid) {
                                let tfmt = format!("#{}", kname);
                                if !shared_book_tags.contains(&tfmt) {
                                    shared_book_tags.push(tfmt);
                                }
                            }
                        }
                    }
                }

                if let Some(aname) = found_author {
                    book_explanation.push(if is_pt {
                        format!("Mesmo autor ({})", aname)
                    } else {
                        format!("Same author ({})", aname)
                    });
                }
                if !shared_book_tags.is_empty() {
                    let tag_str = shared_book_tags.join(", ");
                    book_explanation.push(if is_pt {
                        format!("Temas em comum: {}", tag_str)
                    } else {
                        format!("Shared subjects: {}", tag_str)
                    });
                }

                let default_book_reason = if is_pt {
                    "Afinidade temática literária"
                } else {
                    "Literary thematic affinity"
                };
                let full_book_reason = if book_explanation.is_empty() {
                    default_book_reason.to_string()
                } else {
                    book_explanation.join(" • ")
                };

                let year_str = b
                    .first_publish_year
                    .map(|y| y.to_string())
                    .unwrap_or_else(|| "N/A".to_string());
                let book_subtitle = if is_pt {
                    format!("Livro ({})", year_str)
                } else {
                    format!("Book ({})", year_str)
                };

                results.push(json!({
                    "id": nkey,
                    "title": b.title,
                    "type": "book",
                    "subtitle": book_subtitle,
                    "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                    "reason": full_book_reason,
                }));
                graph_nodes.push(GraphNodePayload {
                    id: nkey.clone(),
                    label: b.title.clone(),
                    node_type: "book".to_string(),
                    val: 12.0,
                    tags: Vec::new(),
                    desc: String::new(),
                });
                book_recs_added += 1;
            }
        }

        // -------------------------------------------------------------------
        // G. REVERSION / BACKFILL: Fill remaining quota slots with best
        // candidates from any domain scoring above MIN_REC_SCORE
        // -------------------------------------------------------------------
        if results.len() < req.limit {
            let needed = req.limit - results.len();
            let mut already_chosen: HashSet<String> = results
                .iter()
                .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .collect();
            for s in &req.seeds {
                already_chosen.insert(s.clone());
            }

            let best_remaining = top_k_from_scores(&state.graph, &p, &exclude_ids, "", Some(needed * 5));
            for item in best_remaining {
                if results.len() >= req.limit {
                    break;
                }
                if item.score < MIN_REC_SCORE {
                    break;
                }
                let nkey = &item.node_key;
                if already_chosen.contains(nkey) {
                    continue;
                }
                let Some((pfx, nid)) = parse_node_key(nkey) else {
                    continue;
                };

                match pfx {
                    "game" => {
                        if let Some(g) = state.catalog.games.get(&nid) {
                            already_chosen.insert(nkey.clone());
                            let year_str = g.release_year.map(|y| y.to_string()).unwrap_or_else(|| "N/A".to_string());
                            let sub = if is_pt { format!("Jogo ({})", year_str) } else { format!("Game ({})", year_str) };

                            let mut comp_name: Option<String> = None;
                            for e in state.graph.out_edges_by_key(nkey) {
                                if let Some(("artist", aid)) = parse_node_key(e.target_key) {
                                    if (e.relation_type == "soundtrack_composer" || e.relation_type == "soundtrack_for") && seed_artists.contains(&aid) {
                                        comp_name = seed_artist_names.get(e.target_key)
                                            .cloned()
                                            .or_else(|| lookup_artist_meta(&conn, aid).map(|(n, _)| n));
                                        if comp_name.is_some() {
                                            break;
                                        }
                                    }
                                }
                            }

                            let reason = if let Some(ref c) = comp_name {
                                if is_pt { format!("Trilha sonora por {}", c) } else { format!("Soundtrack by {}", c) }
                            } else {
                                if is_pt { "Alta ressonância na difusão de estilo e mecânicas".to_string() } else { "High diffusion resonance in mechanics and style".to_string() }
                            };

                            results.push(json!({
                                "id": nkey,
                                "title": g.title,
                                "type": "game",
                                "subtitle": sub,
                                "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                                "reason": reason,
                            }));
                            graph_nodes.push(GraphNodePayload {
                                id: nkey.clone(),
                                label: g.title.clone(),
                                node_type: "game".to_string(),
                                val: 12.0,
                                tags: Vec::new(),
                                desc: String::new(),
                            });
                        }
                    }
                    "book" => {
                        if let Some(b) = state.catalog.books.get(&nid) {
                            already_chosen.insert(nkey.clone());
                            let year_str = b.first_publish_year.map(|y| y.to_string()).unwrap_or_else(|| "N/A".to_string());
                            let sub = if is_pt { format!("Livro ({})", year_str) } else { format!("Book ({})", year_str) };
                            let reason = if is_pt { "Afinidade temática e difusão no grafo".to_string() } else { "Thematic affinity and graph diffusion".to_string() };

                            results.push(json!({
                                "id": nkey,
                                "title": b.title,
                                "type": "book",
                                "subtitle": sub,
                                "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                                "reason": reason,
                            }));
                            graph_nodes.push(GraphNodePayload {
                                id: nkey.clone(),
                                label: b.title.clone(),
                                node_type: "book".to_string(),
                                val: 12.0,
                                tags: Vec::new(),
                                desc: String::new(),
                            });
                        }
                    }
                    "movie" => {
                        if let Some(m) = state.catalog.movies.get(&nid) {
                            already_chosen.insert(nkey.clone());
                            let year_str = m.release_year.map(|y| y.to_string()).unwrap_or_else(|| "N/A".to_string());
                            let sub = if is_pt { format!("Filme ({})", year_str) } else { format!("Movie ({})", year_str) };
                            let reason = if is_pt { "Convergência estética e temática".to_string() } else { "Thematic and aesthetic convergence".to_string() };

                            results.push(json!({
                                "id": nkey,
                                "title": m.title,
                                "type": "movie",
                                "subtitle": sub,
                                "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                                "reason": reason,
                            }));
                            graph_nodes.push(GraphNodePayload {
                                id: nkey.clone(),
                                label: m.title.clone(),
                                node_type: "movie".to_string(),
                                val: 12.5,
                                tags: Vec::new(),
                                desc: String::new(),
                            });
                        }
                    }
                    "series" => {
                        if let Some(s) = state.catalog.series.get(&nid) {
                            already_chosen.insert(nkey.clone());
                            let year_str = s.start_year.map(|y| y.to_string()).unwrap_or_else(|| "N/A".to_string());
                            let sub = if is_pt { format!("Série ({})", year_str) } else { format!("Series ({})", year_str) };
                            let reason = if is_pt { "Afinidade de universo e gênero narrativo".to_string() } else { "Narrative genre and world affinity".to_string() };

                            results.push(json!({
                                "id": nkey,
                                "title": s.title,
                                "type": "series",
                                "subtitle": sub,
                                "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                                "reason": reason,
                            }));
                            graph_nodes.push(GraphNodePayload {
                                id: nkey.clone(),
                                label: s.title.clone(),
                                node_type: "series".to_string(),
                                val: 12.5,
                                tags: Vec::new(),
                                desc: String::new(),
                            });
                        }
                    }
                    "artist" => {
                        if let Some((aname, country)) = lookup_artist_meta(&conn, nid) {
                            already_chosen.insert(nkey.clone());
                            let sub = country
                                .map(|c| if is_pt { format!("Artista • {}", c) } else { format!("Artist • {}", c) })
                                .unwrap_or_else(|| if is_pt { "Artista".to_string() } else { "Artist".to_string() });
                            let reason = if is_pt { "Afinidade no grafo estético".to_string() } else { "Aesthetic graph affinity".to_string() };

                            results.push(json!({
                                "id": nkey,
                                "title": aname,
                                "type": "artist",
                                "subtitle": sub,
                                "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                                "top_track": Value::Null,
                                "reason": reason,
                            }));
                            graph_nodes.push(GraphNodePayload {
                                id: nkey.clone(),
                                label: aname,
                                node_type: "artist".to_string(),
                                val: 12.0,
                                tags: Vec::new(),
                                desc: String::new(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

    // 4. Assemble internal subgraph edges and filter isolated bridges
    {
        let mut extracted_edges: Vec<GraphLinkPayload> = Vec::new();
        let mut seen_links: HashSet<(String, String)> = HashSet::new();

        for i in 0..graph_nodes.len() {
            let u_key = &graph_nodes[i].id;
            for edge in state.graph.out_edges_by_key(u_key) {
                let v_key = edge.target_key;
                if graph_nodes.iter().any(|n| n.id == v_key) {
                    let edge_pair = (u_key.clone(), v_key.to_string());
                    let rev_pair = (v_key.to_string(), u_key.clone());
                    if seen_links.insert(edge_pair) && !seen_links.contains(&rev_pair) {
                        extracted_edges.push(GraphLinkPayload {
                            source: u_key.clone(),
                            target: v_key.to_string(),
                            label: edge.relation_type.to_string(),
                            weight: edge.base_weight,
                        });
                    }
                }
            }
        }

        extracted_edges.sort_by(|a, b| {
            let rel_prio = |rel: &str| {
                if rel.starts_with("soundtrack_") {
                    1
                } else if rel == "is_artist" {
                    2
                } else if rel == "performed_by" {
                    3
                } else if rel == "has_director" {
                    4
                } else if rel == "developed_by" {
                    5
                } else {
                    6
                }
            };
            rel_prio(&a.label).cmp(&rel_prio(&b.label)).then_with(|| {
                b.weight
                    .partial_cmp(&a.weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        graph_links.extend(extracted_edges);

        // Build adjacency map to find all nodes reachable from seeds (eliminate disjoint island components)
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for l in &graph_links {
            adj.entry(&l.source).or_default().push(&l.target);
            adj.entry(&l.target).or_default().push(&l.source);
        }

        let mut reachable_from_seeds: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        for s in &diffusion_seeds {
            if adj.contains_key(s.as_str()) {
                reachable_from_seeds.insert(s.clone());
                queue.push_back(s.as_str());
            }
        }
        for n in &graph_nodes {
            if n.node_type == "seed" && reachable_from_seeds.insert(n.id.clone()) {
                queue.push_back(&n.id);
            }
        }

        while let Some(curr) = queue.pop_front() {
            if let Some(neighbors) = adj.get(curr) {
                for &nxt in neighbors {
                    if reachable_from_seeds.insert(nxt.to_string()) {
                        queue.push_back(nxt);
                    }
                }
            }
        }

        // If there are reachable nodes from seeds, retain only the connected component
        if !reachable_from_seeds.is_empty() {
            graph_nodes.retain(|n| reachable_from_seeds.contains(&n.id));
            graph_links.retain(|l| reachable_from_seeds.contains(&l.source) && reachable_from_seeds.contains(&l.target));
        } else {
            let mut connected_node_ids: HashSet<String> = HashSet::new();
            for l in &graph_links {
                connected_node_ids.insert(l.source.clone());
                connected_node_ids.insert(l.target.clone());
            }
            graph_nodes.retain(|n| connected_node_ids.contains(&n.id) || n.node_type == "seed");
        }
    }

    // 5. Enrich each graph node with descriptive tags and metadata
    {

        for n in &mut graph_nodes {
            let Some((pfx, nid)) = parse_node_key(&n.id) else {
                continue;
            };

            let mut node_tags = collect_node_tags(&state.graph, &state.catalog, &n.id, 12);
            let mut node_desc = String::new();

            match pfx {
                "artist" => {
                    node_desc = if is_pt {
                        "Artista musical".to_string()
                    } else {
                        "Musical Artist".to_string()
                    };
                }
                "track" => {
                    if let Ok((a_id, a_name, bpm)) = conn.query_row(
                        "SELECT a.id, a.name, t.tempo FROM music_tracks t JOIN music_artists a ON a.id = t.artist_id WHERE t.id = ?",
                        params![nid],
                        |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, Option<f64>>(2)?,
                            ))
                        },
                    ) {
                        if node_tags.is_empty() {
                            let art_key = format!("artist:{}", a_id);
                            node_tags =
                                collect_node_tags(&state.graph, &state.catalog, &art_key, 12);
                        }
                        let bpm_str = format!("{} BPM", bpm.unwrap_or(0.0).round() as i32);
                        node_desc = if is_pt {
                            format!("Faixa de {} • {}", a_name, bpm_str)
                        } else {
                            format!("Track by {} • {}", a_name, bpm_str)
                        };
                    }
                }
                "game" => {
                    if let Some(g) = state.catalog.games.get(&nid) {
                        if let Some(y) = g.release_year {
                            node_desc = if is_pt {
                                format!("Jogo ({})", y)
                            } else {
                                format!("Game ({})", y)
                            };
                        }
                    }
                }
                "movie" => {
                    if let Some(m) = state.catalog.movies.get(&nid) {
                        let y_str = m
                            .release_year
                            .map(|y| y.to_string())
                            .unwrap_or_else(|| "N/A".to_string());
                        let r_str = m
                            .vote_average
                            .map(|v| format!("{:.1}", v))
                            .unwrap_or_else(|| "N/A".to_string());
                        node_desc = if is_pt {
                            format!("Filme ({}) • Nota {}", y_str, r_str)
                        } else {
                            format!("Movie ({}) • Rating: {}", y_str, r_str)
                        };
                    }
                }
                "series" => {
                    if let Some(s) = state.catalog.series.get(&nid) {
                        let years_str = match (s.start_year, s.end_year) {
                            (Some(st), Some(e)) if st != e => format!("{}-{}", st, e),
                            (Some(st), _) => st.to_string(),
                            _ => "N/A".to_string(),
                        };
                        let r_str = s
                            .vote_average
                            .map(|v| format!("{:.1}", v))
                            .unwrap_or_else(|| "N/A".to_string());
                        let type_label = if s.series_type == "tvMiniSeries" {
                            if is_pt {
                                "Minissérie"
                            } else {
                                "Mini-Series"
                            }
                        } else {
                            if is_pt {
                                "Série"
                            } else {
                                "Series"
                            }
                        };
                        node_desc = if is_pt {
                            format!("{} ({}) • Nota {}", type_label, years_str, r_str)
                        } else {
                            format!("{} ({}) • Rating: {}", type_label, years_str, r_str)
                        };
                    }
                }
                "book" => {
                    node_desc = if is_pt {
                        "Livro".to_string()
                    } else {
                        "Book".to_string()
                    };
                }
                "developer" => {
                    node_desc = if is_pt {
                        "Estúdio de Desenvolvimento".to_string()
                    } else {
                        "Game Developer Studio".to_string()
                    };
                }
                "keyword" => {
                    node_desc = if is_pt {
                        "Tag Conceitual".to_string()
                    } else {
                        "Conceptual Tag".to_string()
                    };
                }
                "person" => {
                    node_desc = if is_pt {
                        "Diretor / Compositor / Roteirista".to_string()
                    } else {
                        "Director / Composer / Writer".to_string()
                    };
                }
                "author" => {
                    node_desc = if is_pt {
                        "Autor Literário".to_string()
                    } else {
                        "Literary Author".to_string()
                    };
                }
                _ => {}
            }

            n.tags = node_tags;
            n.desc = node_desc;
        }
    }

    let resp_obj = RecommendationResponse {
        seeds: seeds_resolved,
        results,
        graph: GraphPayload {
            nodes: graph_nodes,
            links: graph_links,
        },
    };

    if let Ok(mut cache) = state.cache.lock() {
        cache.insert(cache_key, resp_obj.clone());
    }

    Ok(resp_obj)
}

pub async fn api_recommend(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RecommendationRequest>,
) -> Result<Json<RecommendationResponse>, (StatusCode, String)> {
    tokio::task::spawn_blocking(move || {
        compute_recommendations_sync(&state, &req)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Task join error: {}", e)))?
}

pub fn compute_track_recommendations_sync(
    state: &AppState,
    req: &TrackRecommendRequest,
) -> Result<Vec<TrackRecommendationItem>> {
    if req.seeds.is_empty() {
        return Ok(Vec::new());
    }

    let mut seed_artist_ids: HashSet<i64> = HashSet::new();
    let mut seed_track_ids: HashSet<i64> = HashSet::new();
    let mut seed_audio_features: Vec<(f64, f64, f64, f64)> = Vec::new();

    let conn = state.open_read_conn()?;

    for s in &req.seeds {
        let Some((pfx, id)) = parse_node_key(s) else {
            continue;
        };
        if pfx == "artist" {
            seed_artist_ids.insert(id);
        } else if pfx == "track" {
            seed_track_ids.insert(id);
            if let Ok((a_id, tempo, valence, energy, dance)) = conn.query_row(
                "SELECT artist_id, tempo, valence, energy, danceability FROM music_tracks WHERE id = ?",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<f64>>(1)?,
                        r.get::<_, Option<f64>>(2)?,
                        r.get::<_, Option<f64>>(3)?,
                        r.get::<_, Option<f64>>(4)?,
                    ))
                },
            ) {
                seed_artist_ids.insert(a_id);
                if let Some(t) = tempo {
                    seed_audio_features.push((
                        t,
                        valence.unwrap_or(0.5),
                        energy.unwrap_or(0.5),
                        dance.unwrap_or(0.5),
                    ));
                }
            }
        }
    }

    if seed_artist_ids.is_empty() {
        return Ok(Vec::new());
    }

    let avg_audio = if !seed_audio_features.is_empty() {
        let n = seed_audio_features.len() as f64;
        Some((
            seed_audio_features.iter().map(|f| f.0).sum::<f64>() / n,
            seed_audio_features.iter().map(|f| f.1).sum::<f64>() / n,
            seed_audio_features.iter().map(|f| f.2).sum::<f64>() / n,
            seed_audio_features.iter().map(|f| f.3).sum::<f64>() / n,
        ))
    } else {
        None
    };

    let seed_keys: Vec<String> = seed_artist_ids
        .iter()
        .map(|aid| format!("artist:{}", aid))
        .collect();
    let seed_refs: Vec<&str> = seed_keys.iter().map(|s| s.as_str()).collect();

    let rust_results = run_personalized_pagerank_filtered(
        &state.graph,
        &seed_refs,
        0.85,
        25,
        1e-6,
        Some("artist:"),
        Some(25),
    )?;

    if rust_results.is_empty() {
        return Ok(Vec::new());
    }

    let mut scored_map: HashMap<i64, f64> = HashMap::new();
    for r in &rust_results {
        if let Some(("artist", aid)) = parse_node_key(&r.node_key) {
            scored_map.insert(aid, r.score as f64);
        }
    }

    if scored_map.is_empty() {
        return Ok(Vec::new());
    }

    let art_ids_str = scored_map
        .keys()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let seed_ids_str = seed_artist_ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let sql_candidates = format!(
        "WITH target_artists AS (
            SELECT id, name FROM music_artists WHERE id IN ({art_ids_str})
        ),
        artist_reasons AS (
            SELECT 
                s.target_artist_id AS artist_id,
                'Last.fm co-listening with ' || ma_src.name || ' (' || CAST(ROUND(s.match_score * 100) AS INT) || '%)' AS reason
            FROM artist_similarities s
            JOIN music_artists ma_src ON ma_src.id = s.source_artist_id
            WHERE s.source_artist_id IN ({seed_ids_str})
              AND s.target_artist_id IN ({art_ids_str})
            UNION ALL
            SELECT 
                t2.artist_id,
                'Shared microgenres with ' || ma_src.name || ': ' || STRING_AGG(k.name, ', ') AS reason
            FROM artist_tags t1
            JOIN music_artists ma_src ON ma_src.id = t1.artist_id
            JOIN keywords k ON k.id = t1.keyword_id
            JOIN artist_tags t2 ON t2.keyword_id = k.id AND t2.artist_id IN ({art_ids_str})
            WHERE t1.artist_id IN ({seed_ids_str})
            GROUP BY t2.artist_id, ma_src.name
        ),
        ranked_tracks AS (
            SELECT 
                t.id,
                t.title,
                t.artist_id,
                ta.name AS artist_name,
                alb.title AS album_name,
                t.popularity,
                t.tempo,
                t.valence,
                t.energy,
                t.danceability,
                COALESCE(ar.reason, 'Musical scene diffusion affinity') AS explanation,
                ROW_NUMBER() OVER (PARTITION BY t.artist_id ORDER BY t.popularity DESC) AS rk
            FROM music_tracks t
            JOIN target_artists ta ON ta.id = t.artist_id
            LEFT JOIN music_albums alb ON alb.id = t.album_id
            LEFT JOIN (
                SELECT artist_id, STRING_AGG(reason, ' • ') AS reason 
                FROM artist_reasons 
                GROUP BY artist_id
            ) ar ON ar.artist_id = t.artist_id
            WHERE t.popularity >= {min_pop}
        )
        SELECT id, title, artist_name, album_name, popularity, tempo, valence, energy, danceability, artist_id, explanation
        FROM ranked_tracks
        WHERE rk <= 2
        ORDER BY popularity DESC
        LIMIT 60;",
        art_ids_str = art_ids_str,
        seed_ids_str = seed_ids_str,
        min_pop = req.min_popularity
    );

    let candidates: Vec<(
        i64,
        String,
        String,
        Option<String>,
        i32,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        i64,
        String,
    )> = {
        let mut stmt = conn.prepare(&sql_candidates)?;
        let mut rows = stmt.query([])?;
        let mut list = Vec::new();
        while let Some(r) = rows.next()? {
            list.push((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
            ));
        }
        list
    };

    let max_graph_score = scored_map
        .values()
        .copied()
        .fold(0.0f64, |a, b| if a > b { a } else { b })
        .max(1e-9);

    let mut recommendations = Vec::new();
    for c in candidates {
        let t_id = c.0;
        let a_id = c.9;
        if seed_track_ids.contains(&t_id) {
            continue;
        }

        let g_score = scored_map.get(&a_id).copied().unwrap_or(0.0);
        let graph_norm = (g_score / max_graph_score).clamp(0.0, 1.0);

        let mut audio_affinity = 0.8;
        if let (Some((avg_t, avg_v, avg_e, avg_d)), Some(ct), Some(cv), Some(ce)) =
            (avg_audio, c.5, c.6, c.7)
        {
            let cd = c.8.unwrap_or(0.5);
            let d_valence = (cv - avg_v).powi(2);
            let d_energy = (ce - avg_e).powi(2);
            let d_dance = (cd - avg_d).powi(2);
            let d_tempo = ((ct - avg_t) / 120.0).powi(2);
            let dist = (0.3 * d_valence + 0.3 * d_energy + 0.2 * d_dance + 0.2 * d_tempo).sqrt();
            audio_affinity = (1.0 - dist).max(0.0);
        }

        let final_affinity =
            (1.0 - req.audio_weight) * graph_norm + req.audio_weight * audio_affinity;
        let affinity_percentage = (final_affinity * 1000.0).round() / 10.0;

        recommendations.push(TrackRecommendationItem {
            track_id: t_id,
            title: c.1,
            artist_name: c.2,
            album_name: c.3,
            popularity: c.4,
            affinity_score: affinity_percentage,
            tempo: c.5,
            valence: c.6,
            energy: c.7,
            danceability: c.8,
            explanation: c.10,
        });
    }

    recommendations.sort_by(|a, b| {
        b.affinity_score
            .partial_cmp(&a.affinity_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    recommendations.truncate(req.limit);

    Ok(recommendations)
}

pub async fn api_recommend_tracks(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TrackRecommendRequest>,
) -> Result<Json<Vec<TrackRecommendationItem>>, (StatusCode, String)> {
    tokio::task::spawn_blocking(move || {
        compute_track_recommendations_sync(&state, &req)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Task join error: {}", e)))?
}
