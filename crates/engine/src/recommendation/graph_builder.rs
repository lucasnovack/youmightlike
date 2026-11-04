//! Subgraph anchor initialization, edge extraction, BFS component pruning, and metadata enrichment.

use super::context::is_steam_noise_tag;
use crate::models::{
    collect_node_tags, parse_node_key, AppState, EntityItem, GraphLinkPayload, GraphNodePayload,
};
use duckdb::{params, Connection};
use std::collections::{HashMap, HashSet, VecDeque};

/// Adds seeds, track-artist connections, and structural anchors to the recommendation graph
pub fn init_subgraph_anchors(
    state: &AppState,
    seeds_resolved: &[EntityItem],
    track_seed_artists: &HashMap<String, (i64, String)>,
    seed_tag_counts: &HashMap<i64, usize>,
    diffusion_seeds: &[String],
    graph_nodes: &mut Vec<GraphNodePayload>,
    graph_links: &mut Vec<GraphLinkPayload>,
) {
    for seed_item in seeds_resolved {
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

        // Structural anchors for each seed: up to 1 creator/studio + up to 2 top tags
        // so studio edges never starve a seed of its conceptual tag anchors
        let mut creator_edges = Vec::new();
        let mut tag_edges = Vec::new();

        for e in state.graph.out_edges_by_key(&seed_item.id) {
            match e.relation_type {
                "developed_by" | "has_director" | "has_writer" | "written_by" => {
                    creator_edges.push(e);
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
        for i in 0..per_seed_tags.len() {
            for j in (i + 1)..per_seed_tags.len() {
                let mut pair_shared: Vec<(i64, usize, f32)> = per_seed_tags[i]
                    .iter()
                    .filter_map(|(&kid, &w_i)| {
                        per_seed_tags[j].get(&kid).map(|&w_j| {
                            let global_cnt = seed_tag_counts.get(&kid).copied().unwrap_or(2);
                            (kid, global_cnt, w_i + w_j)
                        })
                    })
                    .collect();

                pair_shared.sort_by(|a, b| {
                    b.1.cmp(&a.1)
                        .then_with(|| {
                            b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .then_with(|| a.0.cmp(&b.0))
                });

                for (kid, _, _) in pair_shared.into_iter().take(2) {
                    if !selected_bridge_kids.contains(&kid) {
                        selected_bridge_kids.push(kid);
                    }
                }
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
    let extract_direct_links =
        |nodes: &[GraphNodePayload], links: &mut Vec<GraphLinkPayload>| {
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

            links.extend(extracted);
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
                            "artist" | "person" | "developer" | "author" => true,
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

/// Enriches each graph node with descriptive tags and bilingual metadata
pub fn enrich_graph_nodes(
    state: &AppState,
    conn: &Connection,
    is_pt: bool,
    graph_nodes: &mut [GraphNodePayload],
) {
    for n in graph_nodes.iter_mut() {
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
                    } else if is_pt {
                        "Série"
                    } else {
                        "Series"
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
