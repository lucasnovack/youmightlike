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
use graph_builder::{
    build_subgraph_and_prune, enrich_graph_nodes, init_subgraph_anchors, resolve_node_metadata,
};
use std::collections::{HashMap, HashSet};

use crate::models::{
    is_tag_relation, lookup_artist_meta, lookup_track_participating_artists, norm_track_title,
    parse_node_key, AppState, EntityItem, GraphLinkPayload, GraphNodePayload, GraphPayload,
    RecommendationRequest, RecommendationResponse,
};
use crate::ppr::{
    penalize_negative_scores, run_personalized_pagerank_multi_hit,
    run_personalized_pagerank_scores_weighted, top_k_from_scores,
};
use serde_json::Value;

/// Runs the full personalized recommendation pipeline and generates the associated explanation subgraph
pub fn run_recommendation_pipeline(
    state: &AppState,
    req: &RecommendationRequest,
) -> Result<RecommendationResponse> {
    if req.seeds.is_empty() {
        return Ok(RecommendationResponse {
            seeds: Vec::new(),
            negative_seeds: Vec::new(),
            known_items_count: req.known_items.len(),
            results: Vec::new(),
            graph: GraphPayload {
                nodes: Vec::new(),
                links: Vec::new(),
            },
        });
    }

    let is_pt = req.lang == "pt";

    let mut sorted_seeds = req.seeds.clone();
    sorted_seeds.sort();
    sorted_seeds.dedup();

    let mut sorted_neg_seeds = req.negative_seeds.clone();
    sorted_neg_seeds.sort();
    sorted_neg_seeds.dedup();

    let mut sorted_known = req.known_items.clone();
    sorted_known.sort();
    sorted_known.dedup();

    let cache_key = format!(
        "{}:{}:{}:{}:{}:{}:{}",
        sorted_seeds.join(","),
        sorted_neg_seeds.join(","),
        sorted_known.join(","),
        req.target_type.as_deref().unwrap_or("all"),
        req.limit,
        req.lang,
        req.beta
    );

    if let Ok(cache) = state.cache.lock() {
        if let Some(cached) = cache.get(&cache_key) {
            return Ok(cached);
        }
    }

    let mut seed_set: HashSet<String> = sorted_seeds.iter().cloned().collect();
    for k in &sorted_known {
        seed_set.insert(k.clone());
    }
    let mut seeds_resolved: Vec<EntityItem> = Vec::new();
    let mut diffusion_seeds: Vec<String> = Vec::new();
    let mut seed_artist_names: HashMap<String, String> = HashMap::new();
    let mut track_seed_artists: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let mut seen_seed_canonical: HashSet<(String, String)> = HashSet::new();

    let conn = state.open_read_conn()?;

    // 1. Resolve seed node metadata
    for s in &sorted_seeds {
        let Some((prefix, nid)) = parse_node_key(s) else {
            continue;
        };
        match prefix {
            "artist" => {
                if let Some((name, country)) = lookup_artist_meta(&conn, nid) {
                    let norm_key = ("artist".to_string(), name.trim().to_lowercase());
                    if !seen_seed_canonical.insert(norm_key) {
                        continue;
                    }
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
                    let norm_key = ("movie".to_string(), m.title.trim().to_lowercase());
                    if !seen_seed_canonical.insert(norm_key) {
                        continue;
                    }
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
                    let norm_key = ("series".to_string(), s_meta.title.trim().to_lowercase());
                    if !seen_seed_canonical.insert(norm_key) {
                        continue;
                    }
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
                    let norm_key = ("game".to_string(), g.title.trim().to_lowercase());
                    if !seen_seed_canonical.insert(norm_key) {
                        continue;
                    }
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
                    let norm_key = ("book".to_string(), b.title.trim().to_lowercase());
                    if !seen_seed_canonical.insert(norm_key) {
                        continue;
                    }
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
                if let Some((t_title, artists)) = lookup_track_participating_artists(&conn, nid) {
                    let norm_key = ("track".to_string(), norm_track_title(&t_title));
                    if !seen_seed_canonical.insert(norm_key) {
                        continue;
                    }
                    let artist_names: Vec<String> =
                        artists.iter().map(|(_, name)| name.clone()).collect();
                    let subtitle = if artist_names.is_empty() {
                        None
                    } else {
                        Some(artist_names.join(", "))
                    };

                    seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name: t_title,
                        entity_type: "track".to_string(),
                        subtitle,
                        extra: None,
                    });

                    track_seed_artists.insert(s.clone(), artists.clone());

                    for (a_id, a_name) in artists {
                        let art_node = format!("artist:{}", a_id);
                        seed_artist_names.insert(art_node.clone(), a_name);
                        if !diffusion_seeds.contains(&art_node) {
                            diffusion_seeds.push(art_node);
                        }
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

    // 2. Resolve negative seeds (anti-seeds / repulsive poles)
    let mut negative_seeds_resolved: Vec<EntityItem> = Vec::new();
    let mut negative_diffusion_seeds: Vec<String> = Vec::new();
    let mut seen_neg_canonical: HashSet<(String, String)> = HashSet::new();

    for s in &sorted_neg_seeds {
        let Some((prefix, nid)) = parse_node_key(s) else {
            continue;
        };
        match prefix {
            "artist" => {
                if let Some((name, country)) = lookup_artist_meta(&conn, nid) {
                    let norm_key = ("artist".to_string(), name.trim().to_lowercase());
                    if !seen_neg_canonical.insert(norm_key) {
                        continue;
                    }
                    negative_seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name,
                        entity_type: "artist".to_string(),
                        subtitle: country,
                        extra: None,
                    });
                    if !negative_diffusion_seeds.contains(s) {
                        negative_diffusion_seeds.push(s.clone());
                    }
                }
            }
            "movie" => {
                if let Some(m) = state.catalog.movies.get(&nid) {
                    let norm_key = ("movie".to_string(), m.title.trim().to_lowercase());
                    if !seen_neg_canonical.insert(norm_key) {
                        continue;
                    }
                    negative_seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name: m.title.clone(),
                        entity_type: "movie".to_string(),
                        subtitle: m.release_year.map(|y| y.to_string()),
                        extra: None,
                    });
                    if !negative_diffusion_seeds.contains(s) {
                        negative_diffusion_seeds.push(s.clone());
                    }
                }
            }
            "series" => {
                if let Some(s_meta) = state.catalog.series.get(&nid) {
                    let norm_key = ("series".to_string(), s_meta.title.trim().to_lowercase());
                    if !seen_neg_canonical.insert(norm_key) {
                        continue;
                    }
                    negative_seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name: s_meta.title.clone(),
                        entity_type: "series".to_string(),
                        subtitle: s_meta.start_year.map(|y| y.to_string()),
                        extra: None,
                    });
                    if !negative_diffusion_seeds.contains(s) {
                        negative_diffusion_seeds.push(s.clone());
                    }
                }
            }
            "game" => {
                if let Some(g) = state.catalog.games.get(&nid) {
                    let norm_key = ("game".to_string(), g.title.trim().to_lowercase());
                    if !seen_neg_canonical.insert(norm_key) {
                        continue;
                    }
                    negative_seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name: g.title.clone(),
                        entity_type: "game".to_string(),
                        subtitle: g.release_year.map(|y| y.to_string()),
                        extra: None,
                    });
                    if !negative_diffusion_seeds.contains(s) {
                        negative_diffusion_seeds.push(s.clone());
                    }
                }
            }
            "book" => {
                if let Some(b) = state.catalog.books.get(&nid) {
                    let norm_key = ("book".to_string(), b.title.trim().to_lowercase());
                    if !seen_neg_canonical.insert(norm_key) {
                        continue;
                    }
                    negative_seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name: b.title.clone(),
                        entity_type: "book".to_string(),
                        subtitle: b.first_publish_year.map(|y| y.to_string()),
                        extra: None,
                    });
                    if !negative_diffusion_seeds.contains(s) {
                        negative_diffusion_seeds.push(s.clone());
                    }
                }
            }
            "track" => {
                if let Some((t_title, artists)) = lookup_track_participating_artists(&conn, nid) {
                    let norm_key = ("track".to_string(), norm_track_title(&t_title));
                    if !seen_neg_canonical.insert(norm_key) {
                        continue;
                    }
                    let artist_names: Vec<String> =
                        artists.iter().map(|(_, name)| name.clone()).collect();
                    let subtitle = if artist_names.is_empty() {
                        None
                    } else {
                        Some(artist_names.join(", "))
                    };

                    negative_seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name: t_title,
                        entity_type: "track".to_string(),
                        subtitle,
                        extra: None,
                    });

                    for (a_id, _) in artists {
                        let art_node = format!("artist:{}", a_id);
                        if !negative_diffusion_seeds.contains(&art_node) {
                            negative_diffusion_seeds.push(art_node);
                        }
                    }

                    if state.graph.get_node_id(s).is_some() && !negative_diffusion_seeds.contains(s)
                    {
                        negative_diffusion_seeds.push(s.clone());
                    }
                }
            }
            "keyword" => {
                if let Some(kname) = state.catalog.keywords.get(&nid) {
                    negative_seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name: format!("#{}", kname),
                        entity_type: "keyword".to_string(),
                        subtitle: Some("Tag".to_string()),
                        extra: None,
                    });
                    if !negative_diffusion_seeds.contains(s) {
                        negative_diffusion_seeds.push(s.clone());
                    }
                }
            }
            "person" => {
                if let Some(pname) = state.catalog.people.get(&nid) {
                    negative_seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name: pname.clone(),
                        entity_type: "person".to_string(),
                        subtitle: Some("Person".to_string()),
                        extra: None,
                    });
                    if !negative_diffusion_seeds.contains(s) {
                        negative_diffusion_seeds.push(s.clone());
                    }
                }
            }
            "developer" => {
                if let Some(dname) = state.catalog.developers.get(&nid) {
                    negative_seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name: dname.clone(),
                        entity_type: "developer".to_string(),
                        subtitle: Some("Studio".to_string()),
                        extra: None,
                    });
                    if !negative_diffusion_seeds.contains(s) {
                        negative_diffusion_seeds.push(s.clone());
                    }
                }
            }
            "author" => {
                if let Some(aname) = state.catalog.authors.get(&nid) {
                    negative_seeds_resolved.push(EntityItem {
                        id: s.clone(),
                        name: aname.clone(),
                        entity_type: "author".to_string(),
                        subtitle: Some("Author".to_string()),
                        extra: None,
                    });
                    if !negative_diffusion_seeds.contains(s) {
                        negative_diffusion_seeds.push(s.clone());
                    }
                }
            }
            _ => {}
        }
    }

    // 3. Run PPR in-memory diffusion with multi-seed cluster balancing
    let active_seeds: Vec<&str> = if diffusion_seeds.is_empty() {
        req.seeds.iter().map(|s| s.as_str()).collect()
    } else {
        diffusion_seeds.iter().map(|s| s.as_str()).collect()
    };

    // Group active seeds by cultural root cluster to prevent multi-track or niche amplification
    let mut cluster_seeds: HashMap<String, Vec<&str>> = HashMap::new();
    for &s in &active_seeds {
        let cluster_key = if s.starts_with("track:") {
            if let Some(artists) = track_seed_artists.get(s) {
                if let Some((first_aid, _)) = artists.first() {
                    format!("artist:{}", first_aid)
                } else {
                    s.to_string()
                }
            } else {
                s.to_string()
            }
        } else {
            s.to_string()
        };
        cluster_seeds.entry(cluster_key).or_default().push(s);
    }

    let num_clusters = cluster_seeds.len().max(1) as f32;
    let cluster_budget = 1.0f32 / num_clusters;
    let mut seed_weights: Vec<(&str, f32)> = Vec::new();

    for (c_key, seeds_in_cluster) in &cluster_seeds {
        let has_parent_artist = seeds_in_cluster.iter().any(|&s| s == c_key);
        let track_count = seeds_in_cluster
            .iter()
            .filter(|&s| s.starts_with("track:"))
            .count();
        if has_parent_artist && track_count > 0 {
            let artist_share = cluster_budget * 0.5;
            let track_share = (cluster_budget * 0.5) / (track_count as f32);
            for &s in seeds_in_cluster {
                if s == c_key {
                    seed_weights.push((s, artist_share));
                } else {
                    seed_weights.push((s, track_share));
                }
            }
        } else {
            let equal_share = cluster_budget / (seeds_in_cluster.len() as f32);
            for &s in seeds_in_cluster {
                seed_weights.push((s, equal_share));
            }
        }
    }

    let mut p = if req.multi_hit {
        run_personalized_pagerank_multi_hit(&state.graph, &seed_weights, 0.85, 25, 1e-6)
            .unwrap_or_else(|_| vec![0.0; state.graph.num_nodes])
    } else {
        run_personalized_pagerank_scores_weighted(&state.graph, &seed_weights, 0.85, 25, 1e-6)
            .unwrap_or_else(|_| vec![0.0; state.graph.num_nodes])
    };

    // Apply repulsive negative feedback diffusion if negative seeds are active
    let active_neg_seeds: Vec<&str> = if negative_diffusion_seeds.is_empty() {
        sorted_neg_seeds.iter().map(|s| s.as_str()).collect()
    } else {
        negative_diffusion_seeds
            .iter()
            .map(|s| s.as_str())
            .collect()
    };

    if !active_neg_seeds.is_empty() {
        let neg_weights: Vec<(&str, f32)> = active_neg_seeds.iter().map(|&s| (s, 1.0f32)).collect();
        if let Ok(p_neg) =
            run_personalized_pagerank_scores_weighted(&state.graph, &neg_weights, 0.85, 25, 1e-6)
        {
            let beta = (req.beta as f32).max(0.1);
            penalize_negative_scores(&mut p, &p_neg, beta);
        }
    }

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
    for s in &sorted_known {
        if let Some(uid) = state.graph.get_node_id(s) {
            exclude_ids.insert(uid);
        }
    }
    for s in &sorted_neg_seeds {
        if let Some(uid) = state.graph.get_node_id(s) {
            exclude_ids.insert(uid);
            p[uid as usize] = 0.0;
        }
    }
    for &s in &active_neg_seeds {
        if let Some(uid) = state.graph.get_node_id(s) {
            exclude_ids.insert(uid);
            p[uid as usize] = 0.0;
        }
    }

    let top_diffused_artists =
        top_k_from_scores(&state.graph, &p, &exclude_ids, "artist:", Some(100));

    let max_diffused_artist_score = top_diffused_artists.first().map(|a| a.score).unwrap_or(0.0);
    let min_artist_aff_cutoff = (max_diffused_artist_score * 0.05).max(1e-6);

    let active_seed_set: HashSet<String> = active_seeds.iter().map(|s| s.to_string()).collect();
    let mut affinitive_artist_keys: HashSet<String> = active_seed_set
        .iter()
        .filter(|s| s.starts_with("artist:"))
        .cloned()
        .collect();

    let mut top_diffused_artist_ids: Vec<i64> = Vec::new();
    for a in top_diffused_artists.iter() {
        if a.score >= min_artist_aff_cutoff {
            affinitive_artist_keys.insert(a.node_key.clone());
            if let Some(("artist", aid)) = parse_node_key(&a.node_key) {
                top_diffused_artist_ids.push(aid);
            }
        }
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
                    && matches!(
                        edge.relation_type,
                        "has_director" | "has_composer" | "has_writer"
                    )
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
        &conn,
        &seeds_resolved,
        &negative_seeds_resolved,
        &track_seed_artists,
        &seed_tag_counts,
        &diffusion_seeds,
        is_pt,
        &mut graph_nodes,
        &mut graph_links,
    );

    // Domain-balanced quotas that adapt dynamically to the user's seed profile
    let (artist_quota, track_quota, movie_quota, series_quota, game_quota, book_quota) =
        match req.target_type.as_deref() {
            Some("music") => {
                let a_q = req.limit.div_ceil(2);
                let t_q = req.limit.saturating_sub(a_q).max(1);
                (a_q, t_q, 0, 0, 0, 0)
            }
            Some("movie") => (0, 0, req.limit, 0, 0, 0),
            Some("series") => (0, 0, 0, req.limit, 0, 0),
            Some("game") => (0, 0, 0, 0, req.limit, 0),
            Some("book") => (0, 0, 0, 0, 0, req.limit),
            _ => {
                // Multi-domain balanced quotas: provide generous pools for each category (~15-20 each)
                // so the user can filter by domain seamlessly in the sidebar without mutating the graph.
                let a_q = 10;
                let t_q = 15;
                let m_q = 15;
                let s_q = 15;
                let g_q = 15;
                let b_q = 5;
                (a_q, t_q, m_q, s_q, g_q, b_q)
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
        top_diffused_artist_ids: &top_diffused_artist_ids,
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

    // 6. Enrich recommendation results with structured metadata
    for item in results.iter_mut() {
        if let Some(id_str) = item.get("id").and_then(|v| v.as_str()) {
            if let Some(node) = graph_nodes.iter().find(|n| n.id == id_str) {
                if let Some(ref meta) = node.metadata {
                    item["metadata"] = meta.clone();
                }
            } else {
                let (_, meta) = resolve_node_metadata(state, &conn, is_pt, id_str);
                if let Some(m) = meta {
                    item["metadata"] = m;
                }
            }
        }
    }

    let resp_obj = RecommendationResponse {
        seeds: seeds_resolved,
        negative_seeds: negative_seeds_resolved,
        known_items_count: sorted_known.len(),
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
