//! Subgraph anchor initialization, edge extraction, BFS component pruning, and metadata enrichment.

use super::context::is_steam_noise_tag;
use crate::models::{
    collect_node_tags, lookup_artist_meta, parse_node_key, AppState, EntityItem, GraphLinkPayload,
    GraphNodePayload,
};
use duckdb::{params, Connection};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};

/// Adds seeds, track-artist connections, and structural anchors to the recommendation graph
pub fn init_subgraph_anchors(
    state: &AppState,
    conn: &Connection,
    seeds_resolved: &[EntityItem],
    _negative_seeds_resolved: &[EntityItem],
    track_seed_artists: &HashMap<String, Vec<(i64, String)>>,
    seed_tag_counts: &HashMap<i64, usize>,
    diffusion_seeds: &[String],
    _is_pt: bool,
    graph_nodes: &mut Vec<GraphNodePayload>,
    graph_links: &mut Vec<GraphLinkPayload>,
) {
    for seed_item in seeds_resolved {
        if !graph_nodes.iter().any(|n| n.id == seed_item.id) {
            graph_nodes.push(GraphNodePayload {
                id: seed_item.id.clone(),
                label: seed_item.name.clone(),
                node_type: "seed".to_string(),
                val: 18.0,
                tags: Vec::new(),
                desc: String::new(),
                metadata: None,
            });
        }
    }

    for seed_item in seeds_resolved {
        if seed_item.entity_type == "track" {
            if let Some(artists) = track_seed_artists.get(&seed_item.id) {
                for (a_id, a_name) in artists {
                    let art_key = format!("artist:{}", a_id);
                    if !graph_nodes.iter().any(|n| n.id == art_key) {
                        graph_nodes.push(GraphNodePayload {
                            id: art_key.clone(),
                            label: a_name.clone(),
                            node_type: "artist".to_string(),
                            val: 14.0,
                            tags: Vec::new(),
                            desc: String::new(),
                            metadata: None,
                        });
                    }
                    if !graph_links.iter().any(|l| {
                        (l.source == seed_item.id && l.target == art_key)
                            || (l.source == art_key && l.target == seed_item.id)
                    }) {
                        graph_links.push(GraphLinkPayload {
                            source: seed_item.id.clone(),
                            target: art_key,
                            label: "performed_by".to_string(),
                            weight: 2.5,
                        });
                    }
                }
            }
        }

        // Structural anchors for each seed:
        // - Up to 1 creator/studio (games/movies/books) OR up to 1 top similar peer artist/game
        // - Up to 2 top non-noise tags (shared multi-seed tags prioritized)
        let mut creator_edges = Vec::new();
        let mut peer_artist_edges = Vec::new();
        let mut peer_game_edges = Vec::new();
        let mut tag_edges = Vec::new();

        for e in state.graph.out_edges_by_key(&seed_item.id) {
            match e.relation_type {
                "developed_by" | "has_director" | "has_writer" | "written_by" => {
                    creator_edges.push(e);
                }
                "similar_to" if seed_item.entity_type == "artist" => {
                    peer_artist_edges.push(e);
                }
                "similar_to" if seed_item.entity_type == "game" => {
                    peer_game_edges.push(e);
                }
                "has_tag" => {
                    if let Some(("keyword", tid)) = parse_node_key(e.target_key) {
                        if let Some(kname) = state.catalog.keywords.get(&tid) {
                            if !is_steam_noise_tag(kname.as_str()) {
                                tag_edges.push((tid, e));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        creator_edges.sort_by(|a, b| {
            let prio = |rel: &str| match rel {
                "developed_by" => 1,
                "has_director" => 2,
                "has_writer" | "written_by" => 3,
                _ => 4,
            };
            prio(a.relation_type)
                .cmp(&prio(b.relation_type))
                .then_with(|| {
                    b.base_weight
                        .partial_cmp(&a.base_weight)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        peer_artist_edges.sort_by(|a, b| {
            b.base_weight
                .partial_cmp(&a.base_weight)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        peer_game_edges.sort_by(|a, b| {
            b.base_weight
                .partial_cmp(&a.base_weight)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        tag_edges.sort_by(|a, b| {
            let cnt_a = seed_tag_counts.get(&a.0).copied().unwrap_or(0);
            let cnt_b = seed_tag_counts.get(&b.0).copied().unwrap_or(0);
            cnt_b
                .cmp(&cnt_a)
                .then_with(|| {
                    b.1.base_weight
                        .partial_cmp(&a.1.base_weight)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.0.cmp(&b.0))
        });

        let selected_anchors = creator_edges
            .into_iter()
            .take(1)
            .chain(peer_artist_edges.into_iter().take(1))
            .chain(peer_game_edges.into_iter().take(1))
            .chain(tag_edges.into_iter().take(2).map(|(_, e)| e));

        for edge in selected_anchors {
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
                    "artist" => lookup_artist_meta(conn, t_nid).map(|(n, _)| n),
                    "game" => state.catalog.games.get(&t_nid).map(|g| g.title.clone()),
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
                            metadata: None,
                        });
                    }
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

    if diffusion_seeds.len() > 1 {
        // 1. Pairwise Multi-Seed Bridge Guarantee:
        // Every pair of seeds that shares non-noise tags gets its top 2 shared tags injected
        // so domain subgraphs (e.g. Skyrim + The Witcher 3) are never starved by other seeds.
        let mut per_seed_tags: Vec<HashMap<i64, f32>> = Vec::with_capacity(diffusion_seeds.len());
        for s_key in diffusion_seeds {
            let mut tags_map = HashMap::new();
            for edge in state.graph.out_edges_by_key(s_key) {
                if edge.relation_type == "has_tag" {
                    if let Some(("keyword", kid)) = parse_node_key(edge.target_key) {
                        if let Some(kname) = state.catalog.keywords.get(&kid) {
                            if !is_steam_noise_tag(kname.as_str()) {
                                tags_map.insert(kid, edge.base_weight);
                            }
                        }
                    }
                }
            }
            per_seed_tags.push(tags_map);
        }

        let mut selected_bridge_kids: Vec<i64> = Vec::new();

        // Score each shared tag by the number of distinct seed pairs it bridges
        let mut bridge_pair_counts: HashMap<i64, (usize, f32)> = HashMap::new();
        for i in 0..per_seed_tags.len() {
            for j in (i + 1)..per_seed_tags.len() {
                for (&kid, &w_i) in &per_seed_tags[i] {
                    if let Some(&w_j) = per_seed_tags[j].get(&kid) {
                        let entry = bridge_pair_counts.entry(kid).or_insert((0, 0.0));
                        entry.0 += 1; // number of seed pairs bridged
                        entry.1 += w_i + w_j;
                    }
                }
            }
        }

        let mut sorted_bridges: Vec<(i64, usize, f32)> = bridge_pair_counts
            .into_iter()
            .map(|(kid, (pairs, w))| (kid, pairs, w))
            .collect();

        // Sort by pair coverage descending, then total weight, then kid deterministic
        sorted_bridges.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.0.cmp(&b.0))
        });

        // Cap to at most 20 pairwise bridges to prevent combinatorial explosion on 30+ seeds
        for (kid, _, _) in sorted_bridges.into_iter().take(20) {
            if !selected_bridge_kids.contains(&kid) {
                selected_bridge_kids.push(kid);
            }
        }

        // 2. Global top shared tags across all seeds (deterministic tie-breaking)
        let mut common_tags: Vec<(i64, usize)> = seed_tag_counts
            .iter()
            .filter(|&(kid, &cnt)| {
                if cnt < 2 {
                    return false;
                }
                if let Some(kname) = state.catalog.keywords.get(kid) {
                    !is_steam_noise_tag(kname.as_str())
                } else {
                    false
                }
            })
            .map(|(&kid, &cnt)| (kid, cnt))
            .collect();
        common_tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        for (kid, _) in common_tags.into_iter().take(5) {
            if !selected_bridge_kids.contains(&kid) {
                selected_bridge_kids.push(kid);
            }
        }

        for kid in selected_bridge_kids {
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
                        metadata: None,
                    });
                }
            }
        }
    }
}

/// Assembles internal subgraph edges, stitches components sharing 2-hop tags, and prunes isolated islands
pub fn build_subgraph_and_prune(
    state: &AppState,
    diffusion_seeds: &[String],
    graph_nodes: &mut Vec<GraphNodePayload>,
    graph_links: &mut Vec<GraphLinkPayload>,
) {
    let extract_direct_links = |nodes: &[GraphNodePayload], links: &mut Vec<GraphLinkPayload>| {
        let mut seen_links: HashSet<(String, String)> = HashSet::new();
        for l in links.iter() {
            seen_links.insert((l.source.clone(), l.target.clone()));
            seen_links.insert((l.target.clone(), l.source.clone()));
        }
        let node_ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        let mut extracted: Vec<GraphLinkPayload> = Vec::new();

        for n in nodes {
            let u_key = &n.id;
            for edge in state.graph.out_edges_by_key(u_key) {
                let v_key = edge.target_key;
                if node_ids.contains(v_key) && u_key != v_key {
                    let edge_pair = (u_key.clone(), v_key.to_string());
                    if seen_links.insert(edge_pair) {
                        seen_links.insert((v_key.to_string(), u_key.clone()));
                        extracted.push(GraphLinkPayload {
                            source: u_key.clone(),
                            target: v_key.to_string(),
                            label: edge.relation_type.to_string(),
                            weight: edge.base_weight,
                        });
                    }
                }
            }
        }

        extracted.sort_by(|a, b| {
            let rel_prio = |rel: &str| {
                if rel == "based_on" || rel == "adapted_as" {
                    1
                } else if rel.starts_with("soundtrack_") {
                    2
                } else if rel == "is_artist" {
                    3
                } else if rel == "similar_to" {
                    4
                } else if rel == "performed_by" {
                    5
                } else if rel == "has_director" {
                    6
                } else if rel == "developed_by" {
                    7
                } else {
                    8
                }
            };
            rel_prio(&a.label).cmp(&rel_prio(&b.label)).then_with(|| {
                b.weight
                    .partial_cmp(&a.weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        // To prevent visual hub monopolies / dense spiderwebs in the visual graph:
        // A single bridge/tag node should not connect to 30+ items.
        // Cap the maximum direct links per hub node (keyword/developer) to the top 5 strongest links.
        let is_hub = |k: &str| k.starts_with("keyword:") || k.starts_with("developer:");
        let mut hub_link_counts: HashMap<String, usize> = HashMap::new();
        let mut filtered_extracted: Vec<GraphLinkPayload> = Vec::with_capacity(extracted.len());

        const MAX_LINKS_PER_HUB: usize = 5;
        for link in extracted {
            let src_hub = is_hub(&link.source);
            let tgt_hub = is_hub(&link.target);

            if src_hub && hub_link_counts.get(&link.source).copied().unwrap_or(0) >= MAX_LINKS_PER_HUB {
                continue;
            }
            if tgt_hub && hub_link_counts.get(&link.target).copied().unwrap_or(0) >= MAX_LINKS_PER_HUB {
                continue;
            }

            if src_hub {
                *hub_link_counts.entry(link.source.clone()).or_insert(0) += 1;
            }
            if tgt_hub {
                *hub_link_counts.entry(link.target.clone()).or_insert(0) += 1;
            }
            filtered_extracted.push(link);
        }

        links.extend(filtered_extracted);
    };

    extract_direct_links(graph_nodes, graph_links);

    // Check if there are multiple disconnected components in graph_nodes that share 2-hop bridges in CSR
    let compute_components =
        |nodes: &[GraphNodePayload], links: &[GraphLinkPayload]| -> Vec<Vec<String>> {
            let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
            for l in links {
                adj.entry(l.source.as_str())
                    .or_default()
                    .push(l.target.as_str());
                adj.entry(l.target.as_str())
                    .or_default()
                    .push(l.source.as_str());
            }
            let mut visited: HashSet<&str> = HashSet::new();
            let mut comps: Vec<Vec<String>> = Vec::new();
            for n in nodes {
                if visited.insert(n.id.as_str()) {
                    let mut comp = Vec::new();
                    let mut q = VecDeque::new();
                    q.push_back(n.id.as_str());
                    while let Some(u) = q.pop_front() {
                        comp.push(u.to_string());
                        if let Some(nbrs) = adj.get(u) {
                            for &v in nbrs {
                                if visited.insert(v) {
                                    q.push_back(v);
                                }
                            }
                        }
                    }
                    comps.push(comp);
                }
            }
            comps
        };

    let comps = compute_components(graph_nodes, graph_links);
    if comps.len() > 1 {
        // Map each candidate bridge target_key -> Vec<(comp_idx, weight)>
        let mut bridge_comp_hits: HashMap<&str, Vec<(usize, f32)>> = HashMap::new();
        for (c_idx, comp) in comps.iter().enumerate() {
            let mut comp_best_for_target: HashMap<&str, f32> = HashMap::new();
            for u_key in comp {
                for edge in state.graph.out_edges_by_key(u_key) {
                    if let Some((tpfx, tid)) = parse_node_key(edge.target_key) {
                        let valid = match tpfx {
                            "keyword" => state
                                .catalog
                                .keywords
                                .get(&tid)
                                .map(|k| !is_steam_noise_tag(k.as_str()))
                                .unwrap_or(false),
                            "artist" | "person" | "developer" | "author" | "game" => true,
                            _ => false,
                        };
                        if valid {
                            let entry = comp_best_for_target.entry(edge.target_key).or_insert(0.0);
                            if edge.base_weight > *entry {
                                *entry = edge.base_weight;
                            }
                        }
                    }
                }
            }
            for (t_key, w) in comp_best_for_target {
                bridge_comp_hits.entry(t_key).or_default().push((c_idx, w));
            }
        }

        // For each pair of components that shares at least one bridge node, add the strongest bridge
        let mut added_bridges = false;
        for i in 0..comps.len() {
            for j in (i + 1)..comps.len() {
                let mut pair_candidates: Vec<(&str, f32)> = bridge_comp_hits
                    .iter()
                    .filter_map(|(&t_key, hits)| {
                        let w_i = hits.iter().find(|&&(c, _)| c == i).map(|&(_, w)| w)?;
                        let w_j = hits.iter().find(|&&(c, _)| c == j).map(|&(_, w)| w)?;
                        Some((t_key, w_i + w_j))
                    })
                    .collect();

                pair_candidates.sort_by(|a, b| {
                    b.1.partial_cmp(&a.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.0.cmp(b.0))
                });

                for (b_key, _) in pair_candidates.into_iter().take(2) {
                    if !graph_nodes.iter().any(|n| n.id == b_key) {
                        if let Some((t_pfx, t_nid)) = parse_node_key(b_key) {
                            let t_label = match t_pfx {
                                "keyword" => state
                                    .catalog
                                    .keywords
                                    .get(&t_nid)
                                    .map(|k| format!("#{}", k)),
                                "developer" => state.catalog.developers.get(&t_nid).cloned(),
                                "person" => state.catalog.people.get(&t_nid).cloned(),
                                "author" => state.catalog.authors.get(&t_nid).cloned(),
                                "game" => state.catalog.games.get(&t_nid).map(|g| g.title.clone()),
                                _ => None,
                            };
                            if let Some(label) = t_label {
                                graph_nodes.push(GraphNodePayload {
                                    id: b_key.to_string(),
                                    label,
                                    node_type: t_pfx.to_string(),
                                    val: 9.0,
                                    tags: Vec::new(),
                                    desc: String::new(),
                                    metadata: None,
                                });
                                added_bridges = true;
                            }
                        }
                    }
                }
            }
        }

        if added_bridges {
            extract_direct_links(graph_nodes, graph_links);
        }
    }

    // Build adjacency map to find all nodes reachable from seeds (eliminate disjoint island components)
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for l in graph_links.iter() {
        adj.entry(&l.source).or_default().push(&l.target);
        adj.entry(&l.target).or_default().push(&l.source);
    }

    let mut reachable_from_seeds: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    for s in diffusion_seeds {
        if adj.contains_key(s.as_str()) {
            reachable_from_seeds.insert(s.clone());
            queue.push_back(s.as_str());
        }
    }
    for n in graph_nodes.iter() {
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
        graph_links.retain(|l| {
            reachable_from_seeds.contains(&l.source) && reachable_from_seeds.contains(&l.target)
        });
    } else {
        let mut connected_node_ids: HashSet<String> = HashSet::new();
        for l in graph_links.iter() {
            connected_node_ids.insert(l.source.clone());
            connected_node_ids.insert(l.target.clone());
        }
        graph_nodes.retain(|n| connected_node_ids.contains(&n.id) || n.node_type == "seed");
    }
}

/// Resolves rich bilingual metadata and structured attributes for any node
pub fn resolve_node_metadata(
    state: &AppState,
    conn: &Connection,
    is_pt: bool,
    node_id: &str,
) -> (String, Option<Value>) {
    let Some((pfx, nid)) = parse_node_key(node_id) else {
        return (String::new(), None);
    };

    match pfx {
        "artist" => {
            let row = conn
                .query_row(
                    "SELECT name, country, type, listeners FROM music_artists WHERE id = ?",
                    params![nid],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, Option<i64>>(3)?,
                        ))
                    },
                )
                .ok();

            if let Some((name, country, artist_type, listeners)) = row {
                let desc = match (&country, is_pt) {
                    (Some(c), true) => format!("Artista musical • {}", c),
                    (Some(c), false) => format!("Musical Artist • {}", c),
                    (None, true) => "Artista musical".to_string(),
                    (None, false) => "Musical Artist".to_string(),
                };
                let meta = json!({
                    "entity_type": "artist",
                    "name": name,
                    "country": country,
                    "artist_type": artist_type,
                    "listeners": listeners,
                });
                (desc, Some(meta))
            } else {
                let desc = if is_pt {
                    "Artista musical".to_string()
                } else {
                    "Musical Artist".to_string()
                };
                (desc, None)
            }
        }
        "track" => {
            let track_row = conn
                .query_row(
                    "SELECT t.title, t.artist_id, ma.name, t.album_id, t.tempo, t.duration_ms, t.popularity, t.spotify_id
                     FROM music_tracks t
                     JOIN music_artists ma ON ma.id = t.artist_id
                     WHERE t.id = ?",
                    params![nid],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<i64>>(3)?,
                            r.get::<_, Option<f64>>(4)?,
                            r.get::<_, Option<i64>>(5)?,
                            r.get::<_, Option<i32>>(6)?,
                            r.get::<_, Option<String>>(7)?,
                        ))
                    },
                )
                .ok();

            if let Some((
                title,
                _primary_aid,
                primary_name,
                album_id,
                tempo_opt,
                duration_ms_opt,
                popularity_opt,
                _spotify_id_opt,
            )) = track_row
            {
                let mut artist_names = vec![primary_name.clone()];
                if let Some(feats) = crate::models::extract_featured_artist_names(&title) {
                    for f in feats {
                        if !artist_names.contains(&f) {
                            artist_names.push(f);
                        }
                    }
                }
                let artists_str = artist_names.join(", ");

                let mut album_title: Option<String> = None;
                let mut release_year: Option<i32> = None;

                if let Some(alb_id) = album_id {
                    if let Ok((at, ay)) = conn.query_row(
                        "SELECT title, release_year FROM music_albums WHERE id = ?",
                        params![alb_id],
                        |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i32>>(1)?)),
                    ) {
                        album_title = at;
                        release_year = ay;
                    }
                }

                let bpm_int = tempo_opt.map(|b| b.round() as i32);
                let duration_str = duration_ms_opt.map(|ms| {
                    let total_secs = ms / 1000;
                    format!("{}:{:02}", total_secs / 60, total_secs % 60)
                });

                let mut parts = Vec::new();
                parts.push(if is_pt {
                    "Faixa".to_string()
                } else {
                    "Track".to_string()
                });
                if !artists_str.is_empty() {
                    parts.push(artists_str.clone());
                }
                match (&album_title, release_year) {
                    (Some(alb), Some(yr)) => parts.push(format!("{} ({})", alb, yr)),
                    (Some(alb), None) => parts.push(alb.clone()),
                    (None, Some(yr)) => parts.push(format!("({})", yr)),
                    (None, None) => {}
                }
                if let Some(bpm) = bpm_int {
                    parts.push(format!("{} BPM", bpm));
                }
                if let Some(ref dur) = duration_str {
                    parts.push(dur.clone());
                }

                let desc = parts.join(" • ");
                let meta = json!({
                    "entity_type": "track",
                    "title": title,
                    "artists": artist_names,
                    "artist_str": artists_str,
                    "album": album_title,
                    "year": release_year,
                    "tempo": bpm_int,
                    "duration_ms": duration_ms_opt,
                    "duration_str": duration_str,
                    "popularity": popularity_opt,
                });

                (desc, Some(meta))
            } else {
                let desc = if is_pt {
                    "Faixa musical".to_string()
                } else {
                    "Music Track".to_string()
                };
                (desc, None)
            }
        }
        "movie" => {
            if let Some(m) = state.catalog.movies.get(&nid) {
                let director: Option<String> = conn
                    .query_row(
                        "SELECT p.name FROM movie_credits c JOIN people p ON p.id = c.person_id WHERE c.movie_id = ? AND c.job = 'Director' LIMIT 1",
                        params![nid],
                        |r| r.get(0),
                    )
                    .ok();

                let mut parts = Vec::new();
                let lbl = if is_pt { "Filme" } else { "Movie" };
                if let Some(y) = m.release_year {
                    parts.push(format!("{} ({})", lbl, y));
                } else {
                    parts.push(lbl.to_string());
                }
                if let Some(ref d) = director {
                    parts.push(format!("Dir. {}", d));
                }
                if let Some(r) = m.vote_average {
                    let r_lbl = if is_pt { "Nota" } else { "Rating" };
                    parts.push(format!("{} {:.1}", r_lbl, r));
                }

                let desc = parts.join(" • ");
                let meta = json!({
                    "entity_type": "movie",
                    "year": m.release_year,
                    "vote_average": m.vote_average,
                    "director": director,
                });
                (desc, Some(meta))
            } else {
                let desc = if is_pt {
                    "Filme".to_string()
                } else {
                    "Movie".to_string()
                };
                (desc, None)
            }
        }
        "series" => {
            if let Some(s) = state.catalog.series.get(&nid) {
                let director: Option<String> = conn
                    .query_row(
                        "SELECT p.name FROM series_credits c JOIN people p ON p.id = c.person_id WHERE c.series_id = ? AND c.job = 'Director' LIMIT 1",
                        params![nid],
                        |r| r.get(0),
                    )
                    .ok();

                let years_str = match (s.start_year, s.end_year) {
                    (Some(st), Some(e)) if st != e => format!("{}-{}", st, e),
                    (Some(st), _) => st.to_string(),
                    _ => String::new(),
                };
                let type_label = if s.series_type == "tvMiniSeries" {
                    if is_pt {
                        "Minissérie"
                    } else {
                        "Mini-Series"
                    }
                } else if is_pt {
                    "Série"
                } else {
                    "Series"
                };

                let mut parts = Vec::new();
                if !years_str.is_empty() {
                    parts.push(format!("{} ({})", type_label, years_str));
                } else {
                    parts.push(type_label.to_string());
                }
                if let Some(ref d) = director {
                    parts.push(format!("Dir. {}", d));
                }
                if let Some(r) = s.vote_average {
                    let r_lbl = if is_pt { "Nota" } else { "Rating" };
                    parts.push(format!("{} {:.1}", r_lbl, r));
                }

                let desc = parts.join(" • ");
                let meta = json!({
                    "entity_type": "series",
                    "series_type": s.series_type,
                    "start_year": s.start_year,
                    "end_year": s.end_year,
                    "years_str": if !years_str.is_empty() { Some(years_str) } else { None },
                    "vote_average": s.vote_average,
                    "director": director,
                });
                (desc, Some(meta))
            } else {
                let desc = if is_pt {
                    "Série".to_string()
                } else {
                    "Series".to_string()
                };
                (desc, None)
            }
        }
        "game" => {
            if let Some(g) = state.catalog.games.get(&nid) {
                let developer: Option<String> = conn
                    .query_row(
                        "SELECT d.name FROM game_dev_links l JOIN game_developers d ON d.id = l.developer_id WHERE l.game_id = ? LIMIT 1",
                        params![nid],
                        |r| r.get(0),
                    )
                    .ok();

                let mut parts = Vec::new();
                let lbl = if is_pt { "Jogo" } else { "Game" };
                if let Some(y) = g.release_year {
                    parts.push(format!("{} ({})", lbl, y));
                } else {
                    parts.push(lbl.to_string());
                }
                if let Some(ref d) = developer {
                    parts.push(d.clone());
                }
                let desc = parts.join(" • ");
                let meta = json!({
                    "entity_type": "game",
                    "year": g.release_year,
                    "developer": developer,
                    "positive_reviews": g.positive_reviews,
                });
                (desc, Some(meta))
            } else {
                let desc = if is_pt {
                    "Jogo".to_string()
                } else {
                    "Game".to_string()
                };
                (desc, None)
            }
        }
        "book" => {
            if let Some(b) = state.catalog.books.get(&nid) {
                let author: Option<String> = conn
                    .query_row(
                        "SELECT a.name FROM book_author_links l JOIN book_authors a ON a.id = l.author_id WHERE l.book_id = ? LIMIT 1",
                        params![nid],
                        |r| r.get(0),
                    )
                    .ok();

                let mut parts = Vec::new();
                let lbl = if is_pt { "Livro" } else { "Book" };
                if let Some(y) = b.first_publish_year {
                    parts.push(format!("{} ({})", lbl, y));
                } else {
                    parts.push(lbl.to_string());
                }
                if let Some(ref a) = author {
                    parts.push(a.clone());
                }
                let desc = parts.join(" • ");
                let meta = json!({
                    "entity_type": "book",
                    "year": b.first_publish_year,
                    "author": author,
                });
                (desc, Some(meta))
            } else {
                let desc = if is_pt {
                    "Livro".to_string()
                } else {
                    "Book".to_string()
                };
                (desc, None)
            }
        }
        "developer" => {
            let desc = if is_pt {
                "Estúdio de Desenvolvimento".to_string()
            } else {
                "Game Developer Studio".to_string()
            };
            (desc, None)
        }
        "keyword" => {
            let desc = if is_pt {
                "Tag Conceitual".to_string()
            } else {
                "Conceptual Tag".to_string()
            };
            (desc, None)
        }
        "person" => {
            let desc = if is_pt {
                "Diretor / Compositor / Roteirista".to_string()
            } else {
                "Director / Composer / Writer".to_string()
            };
            (desc, None)
        }
        "author" => {
            let desc = if is_pt {
                "Autor Literário".to_string()
            } else {
                "Literary Author".to_string()
            };
            (desc, None)
        }
        _ => (String::new(), None),
    }
}

/// Enriches each graph node with descriptive tags, rich summary, and structured metadata
pub fn enrich_graph_nodes(
    state: &AppState,
    conn: &Connection,
    is_pt: bool,
    graph_nodes: &mut [GraphNodePayload],
) {
    for n in graph_nodes.iter_mut() {
        let (desc, meta) = resolve_node_metadata(state, conn, is_pt, &n.id);
        if !desc.is_empty() {
            n.desc = desc;
        }
        n.metadata = meta;

        let mut node_tags = collect_node_tags(&state.graph, &state.catalog, &n.id, 12);
        if node_tags.is_empty() {
            if let Some((pfx, nid)) = parse_node_key(&n.id) {
                if pfx == "track" {
                    if let Ok(a_id) = conn.query_row(
                        "SELECT artist_id FROM music_tracks WHERE id = ?",
                        params![nid],
                        |r| r.get::<_, i64>(0),
                    ) {
                        let art_key = format!("artist:{}", a_id);
                        node_tags = collect_node_tags(&state.graph, &state.catalog, &art_key, 12);
                    }
                }
            }
        }
        n.tags = node_tags;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{GraphCSR, RawEdges};
    use crate::models::{BookMeta, CatalogCache, GameMeta, MovieMeta, ResponseCache, SeriesMeta};
    use std::sync::Mutex;

    fn build_test_state(raw: RawEdges) -> (AppState, Connection) {
        let conn = Connection::open_in_memory().expect("In-memory DuckDB connection");
        let graph = GraphCSR::from_raw(raw, 0.5);
        let catalog = CatalogCache {
            keywords: HashMap::new(),
            people: HashMap::new(),
            developers: HashMap::new(),
            authors: HashMap::new(),
            movies: HashMap::new(),
            series: HashMap::new(),
            games: HashMap::new(),
            books: HashMap::new(),
        };

        let state = AppState {
            db_path: ":memory:".to_string(),
            graph,
            catalog,
            db_conn: Mutex::new(Connection::open_in_memory().unwrap()),
            cache: Mutex::new(ResponseCache::new()),
        };
        (state, conn)
    }

    #[test]
    fn test_subgraph_prunes_isolated_orphan_islands_and_guarantees_degree_gt_zero() {
        let mut raw = RawEdges::default();
        // Component A (reachable from seed movie:1)
        raw.push("movie:1", "keyword:cyberpunk", "has_keyword", 1.0);
        raw.push("movie:1", "movie:2", "similar_to", 0.9);
        raw.push("movie:2", "keyword:cyberpunk", "has_keyword", 0.8);

        // Component B (completely isolated from movie:1)
        raw.push("book:99", "keyword:cooking", "has_tag", 1.0);
        raw.push("book:99", "author:99", "written_by", 1.0);

        let (state, _) = build_test_state(raw);

        let mut graph_nodes = vec![
            GraphNodePayload {
                id: "movie:1".to_string(),
                label: "The Matrix".to_string(),
                node_type: "seed".to_string(),
                val: 18.0,
                tags: vec![],
                desc: String::new(),
                metadata: None,
            },
            GraphNodePayload {
                id: "movie:2".to_string(),
                label: "Dark City".to_string(),
                node_type: "movie".to_string(),
                val: 12.0,
                tags: vec![],
                desc: String::new(),
                metadata: None,
            },
            GraphNodePayload {
                id: "keyword:cyberpunk".to_string(),
                label: "#cyberpunk".to_string(),
                node_type: "keyword".to_string(),
                val: 10.0,
                tags: vec![],
                desc: String::new(),
                metadata: None,
            },
            // Nodes from disconnected island B
            GraphNodePayload {
                id: "book:99".to_string(),
                label: "Cooking 101".to_string(),
                node_type: "book".to_string(),
                val: 12.0,
                tags: vec![],
                desc: String::new(),
                metadata: None,
            },
            GraphNodePayload {
                id: "keyword:cooking".to_string(),
                label: "#cooking".to_string(),
                node_type: "keyword".to_string(),
                val: 10.0,
                tags: vec![],
                desc: String::new(),
                metadata: None,
            },
            GraphNodePayload {
                id: "author:99".to_string(),
                label: "Chef John".to_string(),
                node_type: "author".to_string(),
                val: 10.0,
                tags: vec![],
                desc: String::new(),
                metadata: None,
            },
        ];

        let mut graph_links = Vec::new();
        let diffusion_seeds = vec!["movie:1".to_string()];

        build_subgraph_and_prune(&state, &diffusion_seeds, &mut graph_nodes, &mut graph_links);

        // Island B must be completely pruned
        assert!(
            !graph_nodes.iter().any(|n| n.id.starts_with("book:99")
                || n.id == "keyword:cooking"
                || n.id == "author:99"),
            "Isolated island nodes must be pruned from subgraph"
        );

        // Component A must be preserved
        let remaining_ids: HashSet<String> = graph_nodes.iter().map(|n| n.id.clone()).collect();
        assert!(remaining_ids.contains("movie:1"));
        assert!(remaining_ids.contains("movie:2"));
        assert!(remaining_ids.contains("keyword:cyberpunk"));

        // Strictly verify NO ORPHAN NODES (degree > 0 for all nodes)
        let mut degree_map: HashMap<String, usize> = HashMap::new();
        for l in &graph_links {
            *degree_map.entry(l.source.clone()).or_default() += 1;
            *degree_map.entry(l.target.clone()).or_default() += 1;
        }

        for n in &graph_nodes {
            let deg = degree_map.get(&n.id).copied().unwrap_or(0);
            assert!(
                deg > 0,
                "Node '{}' is an orphan in the rendered subgraph (degree = 0)",
                n.id
            );
        }

        // Verify all links connect nodes that actually exist in graph_nodes
        for l in &graph_links {
            assert!(
                remaining_ids.contains(&l.source),
                "Link source {} missing in graph_nodes",
                l.source
            );
            assert!(
                remaining_ids.contains(&l.target),
                "Link target {} missing in graph_nodes",
                l.target
            );
        }
    }

    #[test]
    fn test_subgraph_stitches_2hop_bridges_between_disconnected_components() {
        let mut raw = RawEdges::default();
        // game:1 and game:2 have no direct edge, but both share developer:10
        raw.push("game:1", "developer:10", "developed_by", 1.0);
        raw.push("game:2", "developer:10", "developed_by", 1.0);

        let (mut state, _) = build_test_state(raw);
        state
            .catalog
            .developers
            .insert(10, "CD Projekt Red".to_string());

        let mut graph_nodes = vec![
            GraphNodePayload {
                id: "game:1".to_string(),
                label: "The Witcher 3".to_string(),
                node_type: "seed".to_string(),
                val: 18.0,
                tags: vec![],
                desc: String::new(),
                metadata: None,
            },
            GraphNodePayload {
                id: "game:2".to_string(),
                label: "Cyberpunk 2077".to_string(),
                node_type: "game".to_string(),
                val: 12.0,
                tags: vec![],
                desc: String::new(),
                metadata: None,
            },
        ];

        let mut graph_links = Vec::new();
        let diffusion_seeds = vec!["game:1".to_string()];

        build_subgraph_and_prune(&state, &diffusion_seeds, &mut graph_nodes, &mut graph_links);

        // developer:10 should have been injected as a bridge stitching game:1 and game:2
        let has_bridge = graph_nodes.iter().any(|n| n.id == "developer:10");
        assert!(
            has_bridge,
            "2-hop shared developer bridge must be injected into graph_nodes"
        );

        // Every node must have degree > 0
        let mut degree_map: HashMap<String, usize> = HashMap::new();
        for l in &graph_links {
            *degree_map.entry(l.source.clone()).or_default() += 1;
            *degree_map.entry(l.target.clone()).or_default() += 1;
        }

        for n in &graph_nodes {
            let deg = degree_map.get(&n.id).copied().unwrap_or(0);
            assert!(
                deg > 0,
                "Node '{}' must be connected after 2-hop stitching",
                n.id
            );
        }
    }

    #[test]
    fn test_bilingual_metadata_resolution_and_integrity() {
        let raw = RawEdges::default();
        let (mut state, conn) = build_test_state(raw);

        // Populate catalog entries
        state.catalog.movies.insert(
            1,
            MovieMeta {
                title: "Blade Runner".to_string(),
                release_year: Some(1982),
                vote_average: Some(8.1),
            },
        );
        state.catalog.series.insert(
            1,
            SeriesMeta {
                title: "Chernobyl".to_string(),
                series_type: "tvMiniSeries".to_string(),
                start_year: Some(2019),
                end_year: Some(2019),
                vote_average: Some(9.4),
            },
        );
        state.catalog.games.insert(
            1,
            GameMeta {
                title: "Portal 2".to_string(),
                release_year: Some(2011),
                positive_reviews: Some(300_000),
            },
        );
        state.catalog.books.insert(
            1,
            BookMeta {
                title: "Neuromancer".to_string(),
                first_publish_year: Some(1984),
            },
        );

        // 1. Movie
        let (desc_pt, meta_pt) = resolve_node_metadata(&state, &conn, true, "movie:1");
        let (desc_en, meta_en) = resolve_node_metadata(&state, &conn, false, "movie:1");
        assert!(desc_pt.contains("Filme (1982)"), "PT: {desc_pt}");
        assert!(desc_pt.contains("Nota 8.1"), "PT: {desc_pt}");
        assert!(desc_en.contains("Movie (1982)"), "EN: {desc_en}");
        assert!(desc_en.contains("Rating 8.1"), "EN: {desc_en}");
        assert_eq!(meta_pt.unwrap()["entity_type"], "movie");
        assert_eq!(meta_en.unwrap()["entity_type"], "movie");

        // 2. Series (Mini-Series)
        let (desc_pt, _) = resolve_node_metadata(&state, &conn, true, "series:1");
        let (desc_en, _) = resolve_node_metadata(&state, &conn, false, "series:1");
        assert!(desc_pt.contains("Minissérie (2019)"), "PT: {desc_pt}");
        assert!(desc_en.contains("Mini-Series (2019)"), "EN: {desc_en}");

        // 3. Game
        let (desc_pt, _) = resolve_node_metadata(&state, &conn, true, "game:1");
        let (desc_en, _) = resolve_node_metadata(&state, &conn, false, "game:1");
        assert!(desc_pt.starts_with("Jogo (2011)"), "PT: {desc_pt}");
        assert!(desc_en.starts_with("Game (2011)"), "EN: {desc_en}");

        // 4. Book
        let (desc_pt, _) = resolve_node_metadata(&state, &conn, true, "book:1");
        let (desc_en, _) = resolve_node_metadata(&state, &conn, false, "book:1");
        assert!(desc_pt.starts_with("Livro (1984)"), "PT: {desc_pt}");
        assert!(desc_en.starts_with("Book (1984)"), "EN: {desc_en}");

        // 5. Fallback pure structural nodes
        let checks = [
            (
                "developer:99",
                "Estúdio de Desenvolvimento",
                "Game Developer Studio",
            ),
            ("keyword:99", "Tag Conceitual", "Conceptual Tag"),
            (
                "person:99",
                "Diretor / Compositor / Roteirista",
                "Director / Composer / Writer",
            ),
            ("author:99", "Autor Literário", "Literary Author"),
            ("movie:99", "Filme", "Movie"),
            ("series:99", "Série", "Series"),
            ("game:99", "Jogo", "Game"),
            ("book:99", "Livro", "Book"),
            ("track:99", "Faixa musical", "Music Track"),
        ];

        for (node_id, exp_pt, exp_en) in checks {
            let (desc_pt, _) = resolve_node_metadata(&state, &conn, true, node_id);
            let (desc_en, _) = resolve_node_metadata(&state, &conn, false, node_id);
            assert_eq!(desc_pt, exp_pt, "Mismatch for PT on {node_id}");
            assert_eq!(desc_en, exp_en, "Mismatch for EN on {node_id}");
        }
    }
}
