//! Books domain recommendation extractor.

use super::super::context::{is_steam_noise_tag, RecContext};
use crate::models::{is_tag_relation, parse_node_key, BookMeta, GraphLinkPayload, GraphNodePayload};
use crate::ppr::top_k_from_scores;
use serde_json::{json, Value};

struct CandidateBook<'a> {
    nkey: String,
    score: f32,
    book: &'a BookMeta,
    best_book_bridge: Option<(String, String, String, f32)>,
    explanation: String,
}

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

    let has_book_seeds = ctx.active_seed_set.iter().any(|s| s.starts_with("book:"));
    let pool_size = (book_quota * 40).max(200);
    let top_books = top_k_from_scores(
        &ctx.state.graph,
        ctx.p,
        ctx.exclude_ids,
        "book:",
        Some(pool_size),
    );

    let mut candidate_books: Vec<CandidateBook> = Vec::new();

    for item in top_books {
        if item.score < ctx.min_rec_score * 0.1 {
            continue;
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
        let mut has_adaptation = false;

        for e in ctx.state.graph.out_edges_by_key(nkey) {
            if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                if (tpfx == "movie" || tpfx == "series")
                    && (e.relation_type == "adapted_as" || e.relation_type == "based_on")
                    && ctx.active_seed_set.contains(e.target_key)
                {
                    has_adaptation = true;
                    let title = if tpfx == "movie" {
                        ctx.state.catalog.movies.get(&tid).map(|m| m.title.clone())
                    } else {
                        ctx.state.catalog.series.get(&tid).map(|s| s.title.clone())
                    };
                    if let Some(scr_title) = title {
                        book_explanation.push(if ctx.is_pt {
                            format!("Livro que originou a adaptação audiovisual '{}'", scr_title)
                        } else {
                            format!("Literary source for '{}'", scr_title)
                        });
                        best_book_bridge =
                            Some((e.target_key.to_string(), scr_title, tpfx.to_string(), 3.5));
                    }
                } else if tpfx == "author"
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
                    && shared_book_tags.len() < 4
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

        if let Some(ref aname) = found_author {
            book_explanation.push(if ctx.is_pt {
                format!("Mesma autoria ({})", aname)
            } else {
                format!("Same author ({})", aname)
            });
        }
        if !shared_book_tags.is_empty() {
            let tag_str = shared_book_tags.join(", ");
            book_explanation.push(if ctx.is_pt {
                format!("Temas literários em comum: {}", tag_str)
            } else {
                format!("Shared literary subjects: {}", tag_str)
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

        let shared_tag_count = shared_book_tags.len();
        let mut boost = 1.0f32;
        if has_adaptation {
            boost *= 3.5;
        }
        if found_author.is_some() {
            boost *= 2.5;
        }
        if !has_book_seeds {
            if shared_tag_count > 0 {
                boost *= 1.0 + (shared_tag_count as f32).powi(2) * 3.0;
            } else if !has_adaptation && found_author.is_none() {
                // Dampen indirect multi-hop gravity without thematic tags
                boost *= 0.15;
            }
        } else if shared_tag_count > 0 {
            boost *= 1.0 + (shared_tag_count as f32) * 0.5;
        }

        let boosted_score = item.score * boost;

        candidate_books.push(CandidateBook {
            nkey: nkey.clone(),
            score: boosted_score,
            book: b,
            best_book_bridge,
            explanation: full_book_reason,
        });
    }

    candidate_books.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut book_recs_added = 0usize;
    for cand in candidate_books {
        if book_recs_added >= book_quota || results.len() >= limit {
            break;
        }

        let year_str = cand
            .book
            .first_publish_year
            .map(|y| y.to_string())
            .unwrap_or_else(|| "N/A".to_string());
        let book_subtitle = if ctx.is_pt {
            format!("Livro ({})", year_str)
        } else {
            format!("Book ({})", year_str)
        };

        results.push(json!({
            "id": cand.nkey,
            "title": cand.book.title,
            "type": "book",
            "subtitle": book_subtitle,
            "score": ((cand.score as f64) * 1000.0 * 100.0).round() / 100.0,
            "reason": cand.explanation,
        }));
        graph_nodes.push(GraphNodePayload {
            id: cand.nkey.clone(),
            label: cand.book.title.clone(),
            node_type: "book".to_string(),
            val: 12.0,
            tags: Vec::new(),
            desc: String::new(),
            metadata: None,
        });

        if let Some((b_key, b_label, b_type, b_weight)) = cand.best_book_bridge {
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
                source: cand.nkey.clone(),
                target: b_key,
                label: if ctx.is_pt {
                    "conectado_a".to_string()
                } else {
                    "connected_to".to_string()
                },
                weight: b_weight,
            });
        }

        // Direct link to seed book if sharing author
        let seed_book_keys: Vec<String> = ctx
            .seed_set
            .iter()
            .filter(|s| s.starts_with("book:"))
            .cloned()
            .collect();
        for s_bkey in &seed_book_keys {
            let shares_author = ctx.state.graph.out_edges_by_key(s_bkey).iter().any(|e| {
                e.relation_type == "written_by"
                    && ctx
                        .state
                        .graph
                        .out_edges_by_key(&cand.nkey)
                        .iter()
                        .any(|ne| ne.target_key == e.target_key)
            });
            if shares_author
                && !graph_links.iter().any(|l| {
                    (l.source == *s_bkey && l.target == cand.nkey)
                        || (l.source == cand.nkey && l.target == *s_bkey)
                })
            {
                graph_links.push(GraphLinkPayload {
                    source: s_bkey.clone(),
                    target: cand.nkey.clone(),
                    label: if ctx.is_pt {
                        "mesmo_autor".to_string()
                    } else {
                        "same_author".to_string()
                    },
                    weight: 3.5,
                });
            }
        }

        book_recs_added += 1;
    }
}
