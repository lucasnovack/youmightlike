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

const MAX_GAMES_PER_DEV_PASS1: usize = 1;
const MAX_GAMES_PER_DEV_PASS2: usize = 2;

struct CandidateGame<'a> {
    nkey: String,
    score: f32,
    game: &'a GameMeta,
    dev_ids: Vec<i64>,
    composer_id: Option<i64>,
    explanation: String,
    bridges: Vec<(String, String, String)>,
}

pub fn extract_games(
    ctx: &RecContext,
    game_quota: usize,
    results: &mut Vec<Value>,
    graph_nodes: &mut Vec<GraphNodePayload>,
    graph_links: &mut Vec<GraphLinkPayload>,
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

    let has_active_keyword_seeds = ctx
        .active_seed_set
        .iter()
        .any(|s| s.starts_with("keyword:"));

    let candidate_pool_size = (game_quota * 180).max(2500);
    let raw_games = top_k_from_scores(
        &ctx.state.graph,
        ctx.p,
        ctx.exclude_ids,
        "game:",
        Some(candidate_pool_size),
    );

    let mut scored_candidates: Vec<CandidateGame> = Vec::new();

    for item in raw_games {
        if item.score < ctx.min_rec_score * 0.2 {
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
        let mut found_composer: Option<(i64, String, bool)> = None;
        let mut found_dev: Option<String> = None;
        let mut dev_ids: Vec<i64> = Vec::new();
        let mut active_seed_tags: Vec<String> = Vec::new();
        let mut specific_tags: Vec<String> = Vec::new();
        let mut fallback_tags: Vec<String> = Vec::new();
        let mut candidate_bridges: Vec<(String, String, String)> = Vec::new();

        let mut touches_direct_seed_artist = false;
        let mut touches_seed_dev = false;
        let mut _touches_seed_tag = false;

        for e in ctx.state.graph.out_edges_by_key(nkey) {
            if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                if tpfx == "artist"
                    && (e.relation_type == "soundtrack_composer"
                        || e.relation_type == "soundtrack_for")
                    && ctx.seed_artists.contains(&tid)
                {
                    let is_direct_artist_seed = ctx.active_seed_set.contains(e.target_key);
                    if is_direct_artist_seed {
                        touches_direct_seed_artist = true;
                    }
                    if found_composer.is_none() {
                        let cname = ctx
                            .seed_artist_names
                            .get(e.target_key)
                            .cloned()
                            .or_else(|| lookup_artist_meta(ctx.conn, tid).map(|(n, _)| n));
                        if let Some(cname) = cname {
                            found_composer = Some((tid, cname, is_direct_artist_seed));
                        }
                    }
                } else if tpfx == "developer" {
                    dev_ids.push(tid);
                    if ctx.seed_devs.contains(&tid) {
                        touches_seed_dev = true;
                        if found_dev.is_none() {
                            found_dev = ctx.state.catalog.developers.get(&tid).cloned();
                            if let Some(ref dname) = found_dev {
                                candidate_bridges.push((
                                    e.target_key.to_string(),
                                    dname.clone(),
                                    "developer".to_string(),
                                ));
                            }
                        }
                    } else if found_dev.is_none() {
                        found_dev = ctx.state.catalog.developers.get(&tid).cloned();
                    }
                } else if tpfx == "keyword" && is_tag_relation(e.relation_type) {
                    let is_active_seed = ctx.active_seed_set.contains(e.target_key);
                    let is_shared_seed_tag = ctx.seed_tag_counts.contains_key(&tid);

                    if is_active_seed {
                        _touches_seed_tag = true;
                    }

                    if is_active_seed || is_shared_seed_tag {
                        if let Some(kname) = ctx.state.catalog.keywords.get(&tid) {
                            let tfmt = format!("#{}", kname);
                            if !is_steam_noise_tag(kname.as_str()) {
                                if is_active_seed {
                                    if !active_seed_tags.contains(&tfmt) {
                                        active_seed_tags.push(tfmt.clone());
                                        candidate_bridges.insert(
                                            0,
                                            (
                                                e.target_key.to_string(),
                                                tfmt,
                                                "keyword".to_string(),
                                            ),
                                        );
                                    }
                                } else if specific_tags.len() < 5 && !specific_tags.contains(&tfmt) {
                                    specific_tags.push(tfmt.clone());
                                    if candidate_bridges.len() < 3 {
                                        candidate_bridges.push((
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

        let active_tag_hits = active_seed_tags.len();
        let shared_tag_hits = specific_tags.len();
        let total_shared_tags = active_tag_hits + shared_tag_hits;

        // If a game only connected via an indirect game composer (e.g. Mikolai Stroinski -> Sniper Ghost Warrior 3)
        // and has fewer than 3 shared thematic tags with the user's profile, skip it!
        if found_composer.is_some()
            && !touches_direct_seed_artist
            && !touches_seed_dev
            && total_shared_tags < 3
        {
            continue;
        }

        let mut boost = 1.0f32;
        if active_tag_hits > 0 {
            // Strong multi-tag confluence boost when user selects #rpg, #open-world, etc.
            boost *= 1.0 + 3.5 * (active_tag_hits as f32).powi(2);
        } else if has_active_keyword_seeds && shared_tag_hits < 4 {
            boost *= 0.25;
        }

        if total_shared_tags > 0 {
            boost *= 1.0 + (total_shared_tags as f32).min(6.0) * 0.45;
        }
        if touches_direct_seed_artist {
            boost *= 1.8;
        }
        if touches_seed_dev && total_shared_tags >= 2 {
            boost *= 1.25;
        } else if found_composer.is_some() && !touches_direct_seed_artist && !touches_seed_dev {
            // Dampen raw PPR inflation from high-weight soundtrack_for edges
            boost *= 0.22;
        }

        // Popular acclaim weighting (prevents obscure 100-review games from outranking Elden Ring / Baldur's Gate 3)
        let rev = g.positive_reviews.unwrap_or(50).max(1) as f32;
        let pop_factor = if rev < 500.0 {
            0.20
        } else if rev < 2_500.0 {
            0.55
        } else {
            1.0 + (rev.ln() - 7.5).max(0.0) * 0.55
        };
        boost *= pop_factor;

        let boosted_score = item.score * boost;

        let mut composer_id_opt = None;
        if let Some((cid, cname, is_direct)) = found_composer {
            composer_id_opt = Some(cid);
            if is_direct || touches_seed_dev {
                game_explanation.push(if ctx.is_pt {
                    format!("Trilha sonora por {}", cname)
                } else {
                    format!("Soundtrack by {}", cname)
                });
                if candidate_bridges.len() < 3 {
                    candidate_bridges.push((
                        format!("artist:{}", cid),
                        cname,
                        "artist".to_string(),
                    ));
                }
            }
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
            composer_id: composer_id_opt,
            explanation: full_game_reason,
            bridges: candidate_bridges,
        });
    }

    scored_candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut game_recs_added = 0usize;
    let mut dev_counts: HashMap<i64, usize> = HashMap::new();
    let mut composer_counts: HashMap<i64, usize> = HashMap::new();
    let mut deferred_candidates: Vec<CandidateGame> = Vec::new();

    // First pass: High studio & composer diversity (max 1 per studio/composer)
    for cand in scored_candidates {
        if game_recs_added >= game_quota {
            break;
        }

        let norm_title = normalize_game_title(&cand.game.title);
        if seen_titles.contains(&norm_title) {
            continue;
        }

        let dev_capped = cand
            .dev_ids
            .iter()
            .any(|&did| dev_counts.get(&did).copied().unwrap_or(0) >= MAX_GAMES_PER_DEV_PASS1);
        let comp_capped = cand
            .composer_id
            .map(|cid| composer_counts.get(&cid).copied().unwrap_or(0) >= 1)
            .unwrap_or(false);

        if dev_capped || comp_capped {
            deferred_candidates.push(cand);
            continue;
        }

        seen_titles.insert(norm_title);
        for &did in &cand.dev_ids {
            *dev_counts.entry(did).or_insert(0) += 1;
        }
        if let Some(cid) = cand.composer_id {
            *composer_counts.entry(cid).or_insert(0) += 1;
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
            metadata: None,
        });

        for (b_id, b_label, b_type) in cand.bridges.into_iter().take(2) {
            if !graph_nodes.iter().any(|n| n.id == b_id) {
                graph_nodes.push(GraphNodePayload {
                    id: b_id.clone(),
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
                target: b_id,
                label: if ctx.is_pt { "conectado_a".to_string() } else { "connected_to".to_string() },
                weight: 2.5,
            });
        }

        // Direct link to seed game if sharing developer
        let seed_game_keys: Vec<String> = ctx.seed_set.iter().filter(|s| s.starts_with("game:")).cloned().collect();
        for s_gkey in &seed_game_keys {
            let shares_dev = ctx.state.graph.out_edges_by_key(s_gkey).iter().any(|e| {
                e.relation_type == "developed_by" && parse_node_key(e.target_key).map(|(_, did)| cand.dev_ids.contains(&did)).unwrap_or(false)
            });
            if shares_dev && !graph_links.iter().any(|l| (l.source == *s_gkey && l.target == cand.nkey) || (l.source == cand.nkey && l.target == *s_gkey)) {
                graph_links.push(GraphLinkPayload {
                    source: s_gkey.clone(),
                    target: cand.nkey.clone(),
                    label: if ctx.is_pt { "mesmo_estúdio".to_string() } else { "same_developer".to_string() },
                    weight: 3.5,
                });
            }
        }
        game_recs_added += 1;
    }

    // Second pass: Allow up to MAX_GAMES_PER_DEV_PASS2 if quota not filled yet
    if game_recs_added < game_quota {
        for cand in deferred_candidates {
            if game_recs_added >= game_quota {
                break;
            }
            let norm_title = normalize_game_title(&cand.game.title);
            if seen_titles.contains(&norm_title) {
                continue;
            }
            let dev_capped = cand
                .dev_ids
                .iter()
                .any(|&did| dev_counts.get(&did).copied().unwrap_or(0) >= MAX_GAMES_PER_DEV_PASS2);
            if dev_capped {
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
                metadata: None,
            });

            for (b_id, b_label, b_type) in cand.bridges.into_iter().take(2) {
                if !graph_nodes.iter().any(|n| n.id == b_id) {
                    graph_nodes.push(GraphNodePayload {
                        id: b_id.clone(),
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
                    target: b_id,
                    label: if ctx.is_pt { "conectado_a".to_string() } else { "connected_to".to_string() },
                    weight: 2.5,
                });
            }

            let seed_game_keys: Vec<String> = ctx.seed_set.iter().filter(|s| s.starts_with("game:")).cloned().collect();
            for s_gkey in &seed_game_keys {
                let shares_dev = ctx.state.graph.out_edges_by_key(s_gkey).iter().any(|e| {
                    e.relation_type == "developed_by" && parse_node_key(e.target_key).map(|(_, did)| cand.dev_ids.contains(&did)).unwrap_or(false)
                });
                if shares_dev && !graph_links.iter().any(|l| (l.source == *s_gkey && l.target == cand.nkey) || (l.source == cand.nkey && l.target == *s_gkey)) {
                    graph_links.push(GraphLinkPayload {
                        source: s_gkey.clone(),
                        target: cand.nkey.clone(),
                        label: if ctx.is_pt { "mesmo_estúdio".to_string() } else { "same_developer".to_string() },
                        weight: 3.5,
                    });
                }
            }
            game_recs_added += 1;
        }
    }
}
