//! Books domain recommendation extractor.

use super::super::context::{is_steam_noise_tag, RecContext};
use crate::models::{is_tag_relation, parse_node_key, GraphLinkPayload, GraphNodePayload};
use crate::ppr::top_k_from_scores;
use serde_json::{json, Value};

pub fn extract_books(
    ctx: &RecContext,
    book_quota: usize,
    limit: usize,
    results: &mut Vec<Value>,
    graph_nodes: &mut Vec<GraphNodePayload>,
    graph_links: &mut Vec<GraphLinkPayload>,
) {
    if book_quota == 0 || results.len() >= limit {
        return;
    }

    let top_books = top_k_from_scores(
        &ctx.state.graph,
        ctx.p,
        ctx.exclude_ids,
        "book:",
        Some(book_quota + 2),
    );
    let mut book_recs_added = 0usize;

    for item in top_books {
        if item.score < ctx.min_rec_score {
            continue;
        }
        if book_recs_added >= book_quota || results.len() >= limit {
            break;
        }
        let nkey = &item.node_key;
        if ctx.seed_set.contains(nkey) {
            continue;
        }
        let Some(("book", nid)) = parse_node_key(nkey) else {
            continue;
        };
        let Some(b) = ctx.state.catalog.books.get(&nid) else {
            continue;
        };

        let mut book_explanation: Vec<String> = Vec::new();
        let mut found_author: Option<String> = None;
        let mut shared_book_tags: Vec<String> = Vec::new();
        let mut best_book_bridge: Option<(String, String, String, f32)> = None;

        for e in ctx.state.graph.out_edges_by_key(nkey) {
            if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                if tpfx == "author"
                    && found_author.is_none()
                    && ctx.seed_authors.contains(&tid)
                {
                    found_author = ctx.state.catalog.authors.get(&tid).cloned();
                    if best_book_bridge.is_none() {
                        if let Some(ref aname) = found_author {
                            best_book_bridge = Some((
                                e.target_key.to_string(),
                                aname.clone(),
                                "author".to_string(),
                                e.base_weight,
                            ));
                        }
                    }
                } else if tpfx == "keyword"
                    && is_tag_relation(e.relation_type)
                    && ctx.seed_tag_counts.contains_key(&tid)
                    && shared_book_tags.len() < 3
                {
                    if let Some(kname) = ctx.state.catalog.keywords.get(&tid) {
                        let tfmt = format!("#{}", kname);
                        if !shared_book_tags.contains(&tfmt) {
                            shared_book_tags.push(tfmt);
                        }
                        if best_book_bridge.is_none() && !is_steam_noise_tag(kname.as_str()) {
                            best_book_bridge = Some((
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

        if let Some(aname) = found_author {
            book_explanation.push(if ctx.is_pt {
                format!("Mesmo autor ({})", aname)
            } else {
                format!("Same author ({})", aname)
            });
        }
        if !shared_book_tags.is_empty() {
            let tag_str = shared_book_tags.join(", ");
            book_explanation.push(if ctx.is_pt {
                format!("Temas em comum: {}", tag_str)
            } else {
                format!("Shared subjects: {}", tag_str)
            });
        }

        let default_book_reason = if ctx.is_pt {
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
        let book_subtitle = if ctx.is_pt {
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

        if let Some((b_key, b_label, b_type, b_weight)) = best_book_bridge {
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
                label: if ctx.is_pt {
                    "conectado_a".to_string()
                } else {
                    "connected_to".to_string()
                },
                weight: b_weight,
            });
        }

        book_recs_added += 1;
    }
}
