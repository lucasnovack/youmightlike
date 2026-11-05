//! Movies domain recommendation extractor.

use super::super::context::{is_steam_noise_tag, RecContext};
use crate::models::{
    is_soundtrack_relation, is_tag_relation, lookup_artist_meta, parse_node_key, GraphLinkPayload,
    GraphNodePayload,
};
use crate::ppr::{top_k_from_scores, ScoredNode};
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn extract_movies(
    ctx: &RecContext,
    movie_quota: usize,
    results: &mut Vec<Value>,
    graph_nodes: &mut Vec<GraphNodePayload>,
    graph_links: &mut Vec<GraphLinkPayload>,
) {
    if movie_quota == 0 {
        return;
    }

    let raw_movies = top_k_from_scores(
        &ctx.state.graph,
        ctx.p,
        ctx.exclude_ids,
        "movie:",
        Some(movie_quota * 8 + 30),
    );

    let mut ranked_movies: Vec<(ScoredNode, Vec<(String, String)>)> = Vec::new();
    for m_item in raw_movies {
        if m_item.score < ctx.min_rec_score {
            continue;
        }
        let mut ost_artists: Vec<(String, String)> = Vec::new();
        let mut seen_ost_names: HashSet<String> = HashSet::new();

        for e in ctx.state.graph.out_edges_by_key(&m_item.node_key) {
            if is_soundtrack_relation(e.relation_type)
                && e.target_key.starts_with("artist:")
                && ctx.affinitive_artist_keys.contains(e.target_key)
            {
                if let Some(("artist", aid)) = parse_node_key(e.target_key) {
                    let aname = ctx
                        .seed_artist_names
                        .get(e.target_key)
                        .cloned()
                        .or_else(|| lookup_artist_meta(ctx.conn, aid).map(|(n, _)| n));
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
        if ctx.seed_set.contains(nkey) {
            continue;
        }
        let Some(("movie", nid)) = parse_node_key(nkey) else {
            continue;
        };
        let Some(m) = ctx.state.catalog.movies.get(&nid) else {
            continue;
        };

        let mut movie_explanation: Vec<String> = Vec::new();
        let mut best_movie_bridge: Option<(String, String, String, f32)> = None;

        if !ost_artists.is_empty() {
            let names: Vec<&str> = ost_artists
                .iter()
                .take(4)
                .map(|(_, n)| n.as_str())
                .collect();
            movie_explanation.push(if ctx.is_pt {
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
                        metadata: None,
                    });
                }
                if best_movie_bridge.is_none() {
                    best_movie_bridge = Some((
                        art_key.clone(),
                        art_name.clone(),
                        "artist".to_string(),
                        3.0,
                    ));
                }
            }
        }

        let mut found_person: Option<String> = None;
        let mut shared_movie_tags: Vec<String> = Vec::new();

        for e in ctx.state.graph.out_edges_by_key(nkey) {
            if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                if tpfx == "person" && found_person.is_none() && ctx.seed_people.contains(&tid) {
                    found_person = ctx.state.catalog.people.get(&tid).cloned();
                    if best_movie_bridge.is_none() {
                        if let Some(ref pname) = found_person {
                            best_movie_bridge = Some((
                                e.target_key.to_string(),
                                pname.clone(),
                                "person".to_string(),
                                e.base_weight,
                            ));
                        }
                    }
                } else if tpfx == "keyword"
                    && is_tag_relation(e.relation_type)
                    && ctx.seed_tag_counts.contains_key(&tid)
                    && shared_movie_tags.len() < 3
                {
                    if let Some(kname) = ctx.state.catalog.keywords.get(&tid) {
                        let tfmt = format!("#{}", kname);
                        if !shared_movie_tags.contains(&tfmt) {
                            shared_movie_tags.push(tfmt);
                        }
                        if best_movie_bridge.is_none() && !is_steam_noise_tag(kname.as_str()) {
                            best_movie_bridge = Some((
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

        if let Some(pname) = found_person {
            movie_explanation.push(if ctx.is_pt {
                format!("Mesmo realizador/compositor ({})", pname)
            } else {
                format!("Same director/composer ({})", pname)
            });
        }
        if !shared_movie_tags.is_empty() {
            let tag_str = shared_movie_tags.join(", ");
            movie_explanation.push(if ctx.is_pt {
                format!("Tags em comum: {}", tag_str)
            } else {
                format!("Shared tags: {}", tag_str)
            });
        }

        let default_movie_reason = if ctx.is_pt {
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
        let movie_subtitle = if ctx.is_pt {
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
            metadata: None,
        });

        if let Some((b_key, b_label, b_type, b_weight)) = best_movie_bridge {
            if !graph_nodes.iter().any(|n| n.id == b_key) {
                graph_nodes.push(GraphNodePayload {
                    id: b_key.clone(),
                    label: b_label,
                    node_type: b_type,
                    val: 9.0,
                    tags: Vec::new(),
                    desc: String::new(),
                    metadata: None,
                });
            }
            graph_links.push(GraphLinkPayload {
                source: nkey.clone(),
                target: b_key,
                label: if ctx.is_pt { "conectado_a".to_string() } else { "connected_to".to_string() },
                weight: b_weight,
            });
        }

        // Direct link to seed movie if sharing director or composer
        let seed_movie_keys: Vec<String> = ctx.seed_set.iter().filter(|s| s.starts_with("movie:")).cloned().collect();
        for s_mkey in &seed_movie_keys {
            let shares_person = ctx.state.graph.out_edges_by_key(s_mkey).iter().any(|e| {
                matches!(e.relation_type, "has_director" | "has_writer" | "soundtrack_composer")
                    && ctx.state.graph.out_edges_by_key(nkey).iter().any(|ne| ne.target_key == e.target_key)
            });
            if shares_person && !graph_links.iter().any(|l| (l.source == *s_mkey && l.target == *nkey) || (l.source == *nkey && l.target == *s_mkey)) {
                graph_links.push(GraphLinkPayload {
                    source: s_mkey.clone(),
                    target: nkey.clone(),
                    label: if ctx.is_pt { "mesmo_realizador".to_string() } else { "same_director".to_string() },
                    weight: 3.5,
                });
            }
        }

        movie_recs_added += 1;
    }
}
