//! Games domain recommendation extractor.

use super::super::context::{is_steam_noise_tag, RecContext};
use crate::models::{
    is_tag_relation, lookup_artist_meta, parse_node_key, GraphLinkPayload, GraphNodePayload,
};
use crate::ppr::top_k_from_scores;
use serde_json::{json, Value};

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

    let top_games = top_k_from_scores(
        &ctx.state.graph,
        ctx.p,
        ctx.exclude_ids,
        "game:",
        Some(game_quota + 4),
    );
    let mut game_recs_added = 0usize;

    for item in top_games {
        if item.score < ctx.min_rec_score {
            continue;
        }
        if game_recs_added >= game_quota {
            break;
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

        let mut game_explanation: Vec<String> = Vec::new();
        let mut found_composer: Option<String> = None;
        let mut found_dev: Option<String> = None;
        let mut specific_tags: Vec<String> = Vec::new();
        let mut fallback_tags: Vec<String> = Vec::new();
        let mut best_game_bridge: Option<(String, String, String)> = None;

        for e in ctx.state.graph.out_edges_by_key(nkey) {
            if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                if tpfx == "artist"
                    && (e.relation_type == "soundtrack_composer"
                        || e.relation_type == "soundtrack_for")
                    && ctx.seed_artists.contains(&tid)
                {
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
                } else if tpfx == "developer"
                    && found_dev.is_none()
                    && ctx.seed_devs.contains(&tid)
                {
                    found_dev = ctx.state.catalog.developers.get(&tid).cloned();
                    if let Some(ref dname) = found_dev {
                        best_game_bridge = Some((
                            e.target_key.to_string(),
                            dname.clone(),
                            "developer".to_string(),
                        ));
                    }
                } else if tpfx == "keyword"
                    && is_tag_relation(e.relation_type)
                    && ctx.seed_tag_counts.contains_key(&tid)
                {
                    if let Some(kname) = ctx.state.catalog.keywords.get(&tid) {
                        let tfmt = format!("#{}", kname);
                        if !is_steam_noise_tag(kname.as_str()) {
                            if specific_tags.len() < 3 && !specific_tags.contains(&tfmt) {
                                specific_tags.push(tfmt);
                            }
                            if best_game_bridge.is_none() {
                                best_game_bridge = Some((
                                e.target_key.to_string(),
                                format!("#{}", kname),
                                "keyword".to_string(),
                            ));
                            }
                        } else if fallback_tags.len() < 2 && !fallback_tags.contains(&tfmt) {
                            fallback_tags.push(tfmt);
                        }
                    }
                }
            }
        }

        if let Some(cname) = found_composer {
            game_explanation.push(if ctx.is_pt {
                format!("Trilha sonora por {}", cname)
            } else {
                format!("Soundtrack by {}", cname)
            });
        }
        if let Some(dname) = found_dev {
            game_explanation.push(if ctx.is_pt {
                format!("Mesmo estúdio ({})", dname)
            } else {
                format!("Same studio ({})", dname)
            });
        }
        if !specific_tags.is_empty() {
            let tag_str = specific_tags.join(", ");
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

        let year_str = g
            .release_year
            .map(|y| y.to_string())
            .unwrap_or_else(|| "N/A".to_string());
        let game_subtitle = if ctx.is_pt {
            format!("Jogo ({})", year_str)
        } else {
            format!("Game ({})", year_str)
        };

        results.push(json!({
            "id": nkey,
            "title": g.title,
            "type": "game",
            "subtitle": game_subtitle,
            "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
            "reason": full_game_reason,
        }));
        graph_nodes.push(GraphNodePayload {
            id: nkey.clone(),
            label: g.title.clone(),
            node_type: "game".to_string(),
            val: 12.0,
            tags: Vec::new(),
            desc: String::new(),
        });
        if let Some((b_id, b_label, b_type)) = best_game_bridge {
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
