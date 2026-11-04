//! Games domain recommendation extractor with franchise deduplication,
//! multi-seed co-occurrence boosting, and studio diversity capping.

use super::super::context::{is_steam_noise_tag, normalize_game_title, RecContext};
use crate::models::{
    is_tag_relation, lookup_artist_meta, parse_node_key, GameMeta, GraphLinkPayload,
    GraphNodePayload,
};
use crate::ppr::top_k_from_scores;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const MAX_GAMES_PER_DEV: usize = 2;

struct CandidateGame<'a> {
    nkey: String,
    score: f32,
    game: &'a GameMeta,
    dev_ids: Vec<i64>,
    explanation: String,
    bridge: Option<(String, String, String)>,
}

pub fn extract_games(
    ctx: &RecContext,
    game_quota: usize,
    results: &mut Vec<Value>,
    graph_nodes: &mut Vec<GraphNodePayload>,
    _graph_links: &mut Vec<GraphLinkPayload>,
) {
    if game_quota == 0 {
        return;
    }

    // Pre-populate seen titles with all seed games to eliminate editions of the seeds
    // (e.g. selecting Skyrim automatically suppresses Skyrim VR and Special Edition)
    let mut seen_titles: HashSet<String> = HashSet::new();
    for seed in ctx.seed_set {
        if let Some(("game", gid)) = parse_node_key(seed) {
            if let Some(g) = ctx.state.catalog.games.get(&gid) {
                seen_titles.insert(normalize_game_title(&g.title));
            }
        }
    }

    let candidate_pool_size = (game_quota * 15).max(120);
    let raw_games = top_k_from_scores(
        &ctx.state.graph,
        ctx.p,
        ctx.exclude_ids,
        "game:",
        Some(candidate_pool_size),
    );

    let mut scored_candidates: Vec<CandidateGame> = Vec::new();

    for item in raw_games {
        if item.score < ctx.min_rec_score {
            continue;
        }
        let nkey = &item.node_key;
        if ctx.seed_set.contains(nkey) {
            continue;
        }
        let Some(("game", nid)) = parse_node_key(nkey) else {
            continue;
        };
        let Some(g) = ctx.state.catalog.games.get(&nid) else {
            continue;
        };

        let norm_title = normalize_game_title(&g.title);
        if seen_titles.contains(&norm_title) {
            continue;
        }

        let mut game_explanation: Vec<String> = Vec::new();
        let mut found_composer: Option<String> = None;
        let mut found_dev: Option<String> = None;
        let mut dev_ids: Vec<i64> = Vec::new();
        let mut active_seed_tags: Vec<String> = Vec::new();
        let mut specific_tags: Vec<String> = Vec::new();
        let mut fallback_tags: Vec<String> = Vec::new();
        let mut best_game_bridge: Option<(String, String, String)> = None;

        let mut touches_seed_artist = false;
        let mut touches_seed_dev = false;
        let mut touches_seed_tag = false;

        for e in ctx.state.graph.out_edges_by_key(nkey) {
            if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                if tpfx == "artist"
                    && (e.relation_type == "soundtrack_composer"
                        || e.relation_type == "soundtrack_for")
                    && ctx.seed_artists.contains(&tid)
                {
                    touches_seed_artist = true;
                    if found_composer.is_none() {
                        let cname = ctx
                            .seed_artist_names
                            .get(e.target_key)
                            .cloned()
                            .or_else(|| lookup_artist_meta(ctx.conn, tid).map(|(n, _)| n));
                        if let Some(cname) = cname {
                            best_game_bridge = Some((
                                e.target_key.to_string(),
                                cname.clone(),
                                "artist".to_string(),
                            ));
                            found_composer = Some(cname);
                        }
                    }
                } else if tpfx == "developer" {
                    dev_ids.push(tid);
                    if ctx.seed_devs.contains(&tid) {
                        touches_seed_dev = true;
                        if found_dev.is_none() {
                            found_dev = ctx.state.catalog.developers.get(&tid).cloned();
                            if let Some(ref dname) = found_dev {
                                best_game_bridge = Some((
                                    e.target_key.to_string(),
                                    dname.clone(),
                                    "developer".to_string(),
                                ));
                            }
                        }
                    } else if found_dev.is_none() {
                        // Capture studio name even if not seed for metadata display
                        found_dev = ctx.state.catalog.developers.get(&tid).cloned();
                    }
                } else if tpfx == "keyword" && is_tag_relation(e.relation_type) {
                    let is_active_seed = ctx.active_seed_set.contains(e.target_key);
                    let is_shared_seed_tag = ctx.seed_tag_counts.contains_key(&tid);

                    if is_active_seed {
                        touches_seed_tag = true;
                    }

                    if is_active_seed || is_shared_seed_tag {
                        if let Some(kname) = ctx.state.catalog.keywords.get(&tid) {
                            let tfmt = format!("#{}", kname);
                            if !is_steam_noise_tag(kname.as_str()) {
                                if is_active_seed {
                                    if !active_seed_tags.contains(&tfmt) {
                                        active_seed_tags.push(tfmt.clone());
                                    }
                                    if best_game_bridge.is_none() {
                                        best_game_bridge = Some((
                                            e.target_key.to_string(),
                                            tfmt,
                                            "keyword".to_string(),
                                        ));
                                    }
                                } else if specific_tags.len() < 4 && !specific_tags.contains(&tfmt) {
                                    specific_tags.push(tfmt.clone());
                                    if best_game_bridge.is_none() {
                                        best_game_bridge = Some((
                                            e.target_key.to_string(),
                                            tfmt,
                                            "keyword".to_string(),
                                        ));
                                    }
                                }
                            } else if fallback_tags.len() < 2 && !fallback_tags.contains(&tfmt) {
                                fallback_tags.push(tfmt);
                            }
                        }
                    }
                }
            }
        }

        // Multi-seed co-occurrence and thematic affinity boosting:
        // When user explicitly selects tags/artists, promote items at the confluence of both seeds
        let mut seed_hits = 0usize;
        if touches_seed_artist {
            seed_hits += 1;
        }
        if touches_seed_dev {
            seed_hits += 1;
        }
        if touches_seed_tag {
            seed_hits += 1;
        }

        let mut boost = 1.0f32;
        if touches_seed_tag {
            // Direct hit on an active seed tag (e.g. open-world)
            boost *= 2.4;
        }
        if touches_seed_artist {
            boost *= 1.35;
        }
        if seed_hits >= 2 {
            // High-value intersection: matches multiple distinct seed concepts simultaneously
            boost *= 1.5;
        }
        let total_shared_tags = active_seed_tags.len() + specific_tags.len();
        if total_shared_tags > 0 {
            boost *= 1.0 + (total_shared_tags as f32).min(5.0) * 0.12;
        }

        let boosted_score = item.score * boost;

        // Compose explanation prioritizing active seed tags
        if let Some(cname) = found_composer {
            game_explanation.push(if ctx.is_pt {
                format!("Trilha sonora por {}", cname)
            } else {
                format!("Soundtrack by {}", cname)
            });
        }
        if let Some(dname) = found_dev {
            if touches_seed_dev {
                game_explanation.push(if ctx.is_pt {
                    format!("Mesmo estúdio ({})", dname)
                } else {
                    format!("Same studio ({})", dname)
                });
            }
        }

        // Combine tags placing active seed tags first
        let mut display_tags: Vec<String> = active_seed_tags;
        for t in specific_tags {
            if display_tags.len() < 4 && !display_tags.contains(&t) {
                display_tags.push(t);
            }
        }
        if !display_tags.is_empty() {
            let tag_str = display_tags.join(", ");
            game_explanation.push(if ctx.is_pt {
                format!("Tags em comum: {}", tag_str)
            } else {
                format!("Shared tags: {}", tag_str)
            });
        } else if !fallback_tags.is_empty() {
            game_explanation.push(format!("Tags: {}", fallback_tags.join(", ")));
        }

        let default_game_reason = if ctx.is_pt {
            "Afinidade de jogabilidade e universo temático"
        } else {
            "Gameplay and thematic universe affinity"
        };
        let full_game_reason = if game_explanation.is_empty() {
            default_game_reason.to_string()
        } else {
            game_explanation.join(" • ")
        };

        scored_candidates.push(CandidateGame {
            nkey: nkey.clone(),
            score: boosted_score,
            game: g,
            dev_ids,
            explanation: full_game_reason,
            bridge: best_game_bridge,
        });
    }

    scored_candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut game_recs_added = 0usize;
    let mut dev_counts: HashMap<i64, usize> = HashMap::new();
    let mut deferred_candidates: Vec<CandidateGame> = Vec::new();

    // First pass: Select diverse games respecting developer studio capping
    for cand in scored_candidates {
        if game_recs_added >= game_quota {
            break;
        }

        let norm_title = normalize_game_title(&cand.game.title);
        if seen_titles.contains(&norm_title) {
            continue;
        }

        // Check if any associated developer studio reached the diversity limit
        let dev_capped = cand
            .dev_ids
            .iter()
            .any(|&did| dev_counts.get(&did).copied().unwrap_or(0) >= MAX_GAMES_PER_DEV);

        if dev_capped {
            deferred_candidates.push(cand);
            continue;
        }

        seen_titles.insert(norm_title);
        for &did in &cand.dev_ids {
            *dev_counts.entry(did).or_insert(0) += 1;
        }

        let year_str = cand
            .game
            .release_year
            .map(|y| y.to_string())
            .unwrap_or_else(|| "N/A".to_string());
        let game_subtitle = if ctx.is_pt {
            format!("Jogo ({})", year_str)
        } else {
            format!("Game ({})", year_str)
        };

        results.push(json!({
            "id": cand.nkey,
            "title": cand.game.title,
            "type": "game",
            "subtitle": game_subtitle,
            "score": ((cand.score as f64) * 1000.0 * 100.0).round() / 100.0,
            "reason": cand.explanation,
        }));
        graph_nodes.push(GraphNodePayload {
            id: cand.nkey.clone(),
            label: cand.game.title.clone(),
            node_type: "game".to_string(),
            val: 12.0,
            tags: Vec::new(),
            desc: String::new(),
        });

        if let Some((b_id, b_label, b_type)) = cand.bridge {
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

    // Second pass: If quota not filled yet, allow deferred games
    if game_recs_added < game_quota {
        for cand in deferred_candidates {
            if game_recs_added >= game_quota {
                break;
            }
            let norm_title = normalize_game_title(&cand.game.title);
            if seen_titles.contains(&norm_title) {
                continue;
            }
            seen_titles.insert(norm_title);

            let year_str = cand
                .game
                .release_year
                .map(|y| y.to_string())
                .unwrap_or_else(|| "N/A".to_string());
            let game_subtitle = if ctx.is_pt {
                format!("Jogo ({})", year_str)
            } else {
                format!("Game ({})", year_str)
            };

            results.push(json!({
                "id": cand.nkey,
                "title": cand.game.title,
                "type": "game",
                "subtitle": game_subtitle,
                "score": ((cand.score as f64) * 1000.0 * 100.0).round() / 100.0,
                "reason": cand.explanation,
            }));
            graph_nodes.push(GraphNodePayload {
                id: cand.nkey.clone(),
                label: cand.game.title.clone(),
                node_type: "game".to_string(),
                val: 12.0,
                tags: Vec::new(),
                desc: String::new(),
            });

            if let Some((b_id, b_label, b_type)) = cand.bridge {
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
}
