use super::super::context::{normalize_game_title, RecContext};
use crate::models::{lookup_artist_meta, parse_node_key, GraphLinkPayload, GraphNodePayload};
use crate::ppr::top_k_from_scores;
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn extract_backfill(
    ctx: &RecContext,
    limit: usize,
    target_type: Option<&str>,
    seeds: &[String],
    results: &mut Vec<Value>,
    graph_nodes: &mut Vec<GraphNodePayload>,
    _graph_links: &mut Vec<GraphLinkPayload>,
) {
    if results.len() >= limit {
        return;
    }

    let needed = limit - results.len();
    let mut already_chosen: HashSet<String> = results
        .iter()
        .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    for s in seeds {
        already_chosen.insert(s.clone());
    }

    let mut seen_game_titles: HashSet<String> = HashSet::new();
    for s in seeds {
        if let Some(("game", gid)) = parse_node_key(s) {
            if let Some(g) = ctx.state.catalog.games.get(&gid) {
                seen_game_titles.insert(normalize_game_title(&g.title));
            }
        }
    }
    for r in results.iter() {
        if let Some(nkey) = r.get("id").and_then(|v| v.as_str()) {
            if let Some(("game", gid)) = parse_node_key(nkey) {
                if let Some(g) = ctx.state.catalog.games.get(&gid) {
                    seen_game_titles.insert(normalize_game_title(&g.title));
                }
            }
        }
    }

    let filter_prefix = match target_type {
        Some("movie") => "movie:",
        Some("series") => "series:",
        Some("game") => "game:",
        Some("book") => "book:",
        Some("music") => "artist:",
        _ => "",
    };

    let best_remaining = top_k_from_scores(
        &ctx.state.graph,
        ctx.p,
        ctx.exclude_ids,
        filter_prefix,
        Some(needed * 10),
    );

    for item in best_remaining {
        if results.len() >= limit {
            break;
        }
        if item.score < ctx.min_rec_score {
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
                if let Some(g) = ctx.state.catalog.games.get(&nid) {
                    let norm = normalize_game_title(&g.title);
                    if seen_game_titles.contains(&norm) {
                        continue;
                    }
                    seen_game_titles.insert(norm);
                    already_chosen.insert(nkey.clone());
                    let year_str = g
                        .release_year
                        .map(|y| y.to_string())
                        .unwrap_or_else(|| "N/A".to_string());
                    let sub = if ctx.is_pt {
                        format!("Jogo ({})", year_str)
                    } else {
                        format!("Game ({})", year_str)
                    };

                    let mut comp_name: Option<String> = None;
                    for e in ctx.state.graph.out_edges_by_key(nkey) {
                        if let Some(("artist", aid)) = parse_node_key(e.target_key) {
                            if (e.relation_type == "soundtrack_composer"
                                || e.relation_type == "soundtrack_for")
                                && ctx.seed_artists.contains(&aid)
                            {
                                comp_name = ctx
                                    .seed_artist_names
                                    .get(e.target_key)
                                    .cloned()
                                    .or_else(|| lookup_artist_meta(ctx.conn, aid).map(|(n, _)| n));
                                if comp_name.is_some() {
                                    break;
                                }
                            }
                        }
                    }

                    let reason = if let Some(ref c) = comp_name {
                        if ctx.is_pt {
                            format!("Trilha sonora por {}", c)
                        } else {
                            format!("Soundtrack by {}", c)
                        }
                    } else if ctx.is_pt {
                        "Alta ressonância na difusão de estilo e mecânicas".to_string()
                    } else {
                        "High diffusion resonance in mechanics and style".to_string()
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
                        metadata: None,
                    });
                }
            }
            "book" => {
                if let Some(b) = ctx.state.catalog.books.get(&nid) {
                    already_chosen.insert(nkey.clone());
                    let year_str = b
                        .first_publish_year
                        .map(|y| y.to_string())
                        .unwrap_or_else(|| "N/A".to_string());
                    let sub = if ctx.is_pt {
                        format!("Livro ({})", year_str)
                    } else {
                        format!("Book ({})", year_str)
                    };
                    let reason = if ctx.is_pt {
                        "Afinidade temática e difusão no grafo".to_string()
                    } else {
                        "Thematic affinity and graph diffusion".to_string()
                    };

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
                        metadata: None,
                    });
                }
            }
            "movie" => {
                if let Some(m) = ctx.state.catalog.movies.get(&nid) {
                    already_chosen.insert(nkey.clone());
                    let year_str = m
                        .release_year
                        .map(|y| y.to_string())
                        .unwrap_or_else(|| "N/A".to_string());
                    let sub = if ctx.is_pt {
                        format!("Filme ({})", year_str)
                    } else {
                        format!("Movie ({})", year_str)
                    };
                    let reason = if ctx.is_pt {
                        "Convergência estética e temática".to_string()
                    } else {
                        "Thematic and aesthetic convergence".to_string()
                    };

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
                        metadata: None,
                    });
                }
            }
            "series" => {
                if let Some(s) = ctx.state.catalog.series.get(&nid) {
                    already_chosen.insert(nkey.clone());
                    let year_str = s
                        .start_year
                        .map(|y| y.to_string())
                        .unwrap_or_else(|| "N/A".to_string());
                    let sub = if ctx.is_pt {
                        format!("Série ({})", year_str)
                    } else {
                        format!("Series ({})", year_str)
                    };
                    let reason = if ctx.is_pt {
                        "Afinidade de universo e gênero narrativo".to_string()
                    } else {
                        "Narrative genre and world affinity".to_string()
                    };

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
                        metadata: None,
                    });
                }
            }
            "artist" => {
                if let Some((aname, country)) = lookup_artist_meta(ctx.conn, nid) {
                    already_chosen.insert(nkey.clone());
                    let sub = country
                        .map(|c| {
                            if ctx.is_pt {
                                format!("Artista • {}", c)
                            } else {
                                format!("Artist • {}", c)
                            }
                        })
                        .unwrap_or_else(|| {
                            if ctx.is_pt {
                                "Artista".to_string()
                            } else {
                                "Artist".to_string()
                            }
                        });
                    let reason = if ctx.is_pt {
                        "Afinidade no grafo estético".to_string()
                    } else {
                        "Aesthetic graph affinity".to_string()
                    };

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
                        metadata: None,
                    });
                }
            }
            _ => {}
        }
    }
}
