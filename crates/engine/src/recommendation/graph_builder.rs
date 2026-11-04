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

        // Structural anchors for each seed (filtering infrastructure noise)
        let mut anchor_edges = state.graph.out_edges_by_key(&seed_item.id);
        anchor_edges.retain(|e| {
            if let Some(("keyword", tid)) = parse_node_key(e.target_key) {
                if let Some(kname) = state.catalog.keywords.get(&tid) {
                    return !is_steam_noise_tag(kname.as_str());
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
                    !is_steam_noise_tag(kname.as_str())
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
}

/// Assembles internal subgraph edges and prunes disconnected island components via BFS from seeds
pub fn build_subgraph_and_prune(
    state: &AppState,
    diffusion_seeds: &[String],
    graph_nodes: &mut Vec<GraphNodePayload>,
    graph_links: &mut Vec<GraphLinkPayload>,
) {
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
