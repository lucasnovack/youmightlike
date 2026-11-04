//! Recommendation pipeline and domain orchestration.

pub mod context;
pub mod extractors;
pub mod graph_builder;
pub mod tracks;

use anyhow::Result;
use context::{RecContext, MIN_REC_SCORE};
use extractors::{
    extract_backfill, extract_books, extract_games, extract_movies, extract_music, extract_series,
};
use graph_builder::{build_subgraph_and_prune, enrich_graph_nodes, init_subgraph_anchors};
use std::collections::{HashMap, HashSet};

use crate::models::{
    is_tag_relation, lookup_artist_meta, parse_node_key, AppState, EntityItem, GraphLinkPayload,
    GraphNodePayload, GraphPayload, RecommendationRequest, RecommendationResponse,
};
use crate::ppr::{run_personalized_pagerank_scores, top_k_from_scores};
use duckdb::params;
use serde_json::Value;

/// Runs the full personalized recommendation pipeline and generates the associated explanation subgraph
pub fn run_recommendation_pipeline(
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

    let top_diffused_artists =
        top_k_from_scores(&state.graph, &p, &exclude_ids, "artist:", Some(100));

    let active_seed_set: HashSet<String> = active_seeds.iter().map(|s| s.to_string()).collect();
    let mut affinitive_artist_keys: HashSet<String> = active_seed_set
        .iter()
        .filter(|s| s.starts_with("artist:"))
        .cloned()
        .collect();
    for a in top_diffused_artists.iter().take(50) {
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
                } else if tpfx == "artist"
                    && matches!(
                        edge.relation_type,
                        "soundtrack_composer" | "soundtrack_featured" | "is_artist"
                    )
                {
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

    // 3. Add seed nodes and structural anchors to subgraph
    init_subgraph_anchors(
        state,
        &seeds_resolved,
        &track_seed_artists,
        &seed_tag_counts,
        &diffusion_seeds,
        &mut graph_nodes,
        &mut graph_links,
    );

    // Domain-balanced quotas that adapt dynamically to the user's seed profile
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
                let mut game_affinity_seeds = 0usize;
                let mut music_affinity_seeds = 0usize;
                let mut screen_affinity_seeds = 0usize;

                for s in &seeds_resolved {
                    match s.entity_type.as_str() {
                        "game" | "developer" => game_affinity_seeds += 2,
                        "artist" | "track" => music_affinity_seeds += 2,
                        "movie" | "series" | "person" => screen_affinity_seeds += 2,
                        "keyword" => {
                            let edges = state.graph.out_edges_by_key(&s.id);
                            if edges.iter().take(40).any(|e| e.target_key.starts_with("game:")) {
                                game_affinity_seeds += 2;
                            }
                            if edges.iter().take(40).any(|e| e.target_key.starts_with("artist:")) {
                                music_affinity_seeds += 1;
                            }
                            if edges
                                .iter()
                                .take(40)
                                .any(|e| e.target_key.starts_with("movie:") || e.target_key.starts_with("series:"))
                            {
                                screen_affinity_seeds += 1;
                            }
                        }
                        _ => {}
                    }
                }

                let total_slots = req.limit.max(16);
                let g_q = if game_affinity_seeds >= 4 {
                    (total_slots * 40 / 100).max(6)
                } else if game_affinity_seeds >= 2 {
                    (total_slots * 30 / 100).max(4)
                } else {
                    (total_slots * 15 / 100).max(2)
                };

                let rem = total_slots.saturating_sub(g_q);
                let a_q = if music_affinity_seeds > 0 {
                    (rem * 32 / 100).max(3)
                } else {
                    (rem * 24 / 100).max(2)
                };
                let t_q = if music_affinity_seeds > 0 {
                    (rem * 28 / 100).max(2)
                } else {
                    (rem * 20 / 100).max(2)
                };
                let m_q = if screen_affinity_seeds > 0 {
                    (rem * 25 / 100).max(3)
                } else {
                    (rem * 20 / 100).max(2)
                };
                let s_q = rem.saturating_sub(a_q + t_q + m_q).max(2);
                (a_q, t_q, m_q, s_q, g_q, 1)
            }
        };

    let is_pt = req.lang == "pt";

    let ctx = RecContext {
        state,
        conn: &conn,
        p: &p,
        exclude_ids: &exclude_ids,
        seed_set: &seed_set,
        active_seed_set: &active_seed_set,
        affinitive_artist_keys: &affinitive_artist_keys,
        seed_artist_names: &seed_artist_names,
        seed_tag_counts: &seed_tag_counts,
        seed_people: &seed_people,
        seed_devs: &seed_devs,
        seed_authors: &seed_authors,
        seed_artists: &seed_artists,
        seed_games: &seed_games,
        is_pt,
        min_rec_score: MIN_REC_SCORE,
    };

    // Domain recommendation extraction
    extract_music(
        &ctx,
        artist_quota,
        track_quota,
        &mut results,
        &mut graph_nodes,
        &mut graph_links,
    );
    extract_movies(
        &ctx,
        movie_quota,
        &mut results,
        &mut graph_nodes,
        &mut graph_links,
    );
    extract_series(
        &ctx,
        series_quota,
        &mut results,
        &mut graph_nodes,
        &mut graph_links,
    );
    extract_games(
        &ctx,
        game_quota,
        &mut results,
        &mut graph_nodes,
        &mut graph_links,
    );
    extract_books(
        &ctx,
        book_quota,
        req.limit,
        &mut results,
        &mut graph_nodes,
        &mut graph_links,
    );
    extract_backfill(
        &ctx,
        req.limit,
        req.target_type.as_deref(),
        &req.seeds,
        &mut results,
        &mut graph_nodes,
        &mut graph_links,
    );

    // 4. Assemble internal subgraph edges and filter isolated bridges via BFS
    build_subgraph_and_prune(state, &diffusion_seeds, &mut graph_nodes, &mut graph_links);

    // 5. Enrich each graph node with descriptive tags and metadata
    enrich_graph_nodes(state, &conn, is_pt, &mut graph_nodes);

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
