//! Music domain recommendation extractor (artists and tracks).

use super::super::context::{is_steam_noise_tag, RecContext};
use crate::models::{
    is_tag_relation, lookup_artist_meta, lookup_top_track_full, parse_node_key, GraphLinkPayload,
    GraphNodePayload, TopTrackRow,
};
use crate::ppr::top_k_from_scores;
use duckdb::params;
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn extract_music(
    ctx: &RecContext,
    artist_quota: usize,
    track_quota: usize,
    results: &mut Vec<Value>,
    graph_nodes: &mut Vec<GraphNodePayload>,
    graph_links: &mut Vec<GraphLinkPayload>,
) {
    let mut recommended_artist_tracks: Vec<(i64, String, f32, TopTrackRow, String)> = Vec::new();
    let mut artist_recs_added = 0usize;

    if artist_quota > 0 || track_quota > 0 {
        let mut seen_artist_names: HashSet<String> = ctx
            .seed_artist_names
            .values()
            .map(|s| s.trim().to_lowercase())
            .collect();
        if seen_artist_names.contains("kanye west") || seen_artist_names.contains("ye") {
            seen_artist_names.insert("kanye west".to_string());
            seen_artist_names.insert("ye".to_string());
        }

        let top_diffused_artists =
            top_k_from_scores(&ctx.state.graph, ctx.p, ctx.exclude_ids, "artist:", Some(30));

        for item in &top_diffused_artists {
            if item.score < ctx.min_rec_score {
                continue;
            }
            let nkey = &item.node_key;
            if ctx.seed_set.contains(nkey) {
                continue;
            }
            let out_edges = ctx.state.graph.out_edges_by_key(nkey);
            if out_edges.len() < 5
                && !out_edges.iter().any(|e| {
                    e.relation_type == "soundtrack_composer" || e.relation_type == "soundtrack_for"
                })
            {
                continue;
            }
            let Some(("artist", nid)) = parse_node_key(nkey) else {
                continue;
            };

            let Some((aname, country)) = lookup_artist_meta(ctx.conn, nid) else {
                continue;
            };
            let aname_norm = aname
                .trim()
                .to_lowercase()
                .replace('’', "'")
                .replace(['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}'], "-");
            if !seen_artist_names.insert(aname_norm) {
                continue;
            }

            // In-memory path explanation via directed CSR edges
            let mut explanation_parts: Vec<String> = Vec::new();
            let mut direct_co_listening = false;
            let mut shared_tags: Vec<String> = Vec::new();
            let mut best_artist_bridge: Option<(String, String, String, f32)> = None;

            for e in out_edges {
                if (e.relation_type == "soundtrack_composer" || e.relation_type == "soundtrack_for")
                    && ctx.seed_games.contains(&parse_node_key(e.target_key).map(|(_, id)| id).unwrap_or(0))
                {
                    if let Some(("game", gid)) = parse_node_key(e.target_key) {
                        if let Some(g) = ctx.state.catalog.games.get(&gid) {
                            explanation_parts.push(if ctx.is_pt {
                                format!("Compositor da trilha de {}", g.title)
                            } else {
                                format!("Soundtrack composer for {}", g.title)
                            });
                            if best_artist_bridge.is_none() {
                                best_artist_bridge = Some((
                                    e.target_key.to_string(),
                                    g.title.clone(),
                                    "game".to_string(),
                                    e.base_weight,
                                ));
                            }
                        }
                    }
                } else if e.relation_type == "similar_to"
                    && ctx.active_seed_set.contains(e.target_key)
                    && !direct_co_listening
                {
                    if let Some(("artist", s_aid)) = parse_node_key(e.target_key) {
                        let s_name = ctx
                            .seed_artist_names
                            .get(e.target_key)
                            .cloned()
                            .or_else(|| lookup_artist_meta(ctx.conn, s_aid).map(|(n, _)| n));
                        if let Some(sname) = s_name {
                            explanation_parts.push(if ctx.is_pt {
                                format!("Similaridade direta com {}", sname)
                            } else {
                                format!("Direct similarity with {}", sname)
                            });
                            direct_co_listening = true;
                            if best_artist_bridge.is_none() {
                                best_artist_bridge = Some((
                                    e.target_key.to_string(),
                                    sname,
                                    "artist".to_string(),
                                    e.base_weight,
                                ));
                            }
                        }
                    }
                } else if is_tag_relation(e.relation_type) && e.target_key.starts_with("keyword:")
                {
                    if let Some(("keyword", kid)) = parse_node_key(e.target_key) {
                        if ctx.seed_tag_counts.contains_key(&kid) && shared_tags.len() < 3 {
                            if let Some(kname) = ctx.state.catalog.keywords.get(&kid) {
                                let tfmt = format!("#{}", kname);
                                if !shared_tags.contains(&tfmt) {
                                    shared_tags.push(tfmt);
                                }
                                if best_artist_bridge.is_none() && !is_steam_noise_tag(kname.as_str()) {
                                    best_artist_bridge = Some((
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
            }

            if !shared_tags.is_empty() {
                let tags_joined = shared_tags.join(", ");
                explanation_parts.push(if ctx.is_pt {
                    format!("Tags compartilhadas: {}", tags_joined)
                } else {
                    format!("Shared tags: {}", tags_joined)
                });
            }

            let default_reason = if ctx.is_pt {
                "Afinidade e proximidade no grafo musical"
            } else {
                "Graph diffusion affinity and proximity"
            };
            let full_reason = if explanation_parts.is_empty() {
                default_reason.to_string()
            } else {
                explanation_parts.join(" • ")
            };

            let top_track_row = lookup_top_track_full(ctx.conn, nid);
            let top_track_val = top_track_row.as_ref().map(|t| {
                json!({
                    "title": t.title,
                    "popularity": t.popularity.unwrap_or(50),
                })
            });

            if let Some(ref trk) = top_track_row {
                recommended_artist_tracks.push((
                    nid,
                    aname.clone(),
                    item.score,
                    trk.clone(),
                    full_reason.clone(),
                ));
            }

            if artist_recs_added < artist_quota {
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

                results.push(json!({
                    "id": nkey,
                    "title": aname,
                    "type": "artist",
                    "subtitle": sub,
                    "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                    "top_track": top_track_val,
                    "reason": full_reason,
                }));

                graph_nodes.push(GraphNodePayload {
                    id: nkey.clone(),
                    label: aname.clone(),
                    node_type: "artist".to_string(),
                    val: 12.0,
                    tags: Vec::new(),
                    desc: String::new(),
                });

                if let Some((b_key, b_label, b_type, b_weight)) = best_artist_bridge {
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
                        label: if ctx.is_pt { "conectado_a".to_string() } else { "connected_to".to_string() },
                        weight: b_weight,
                    });
                }

                artist_recs_added += 1;
            }
        }
    }

    // Recommended Tracks (`track:*`)
    if track_quota > 0 {
        let mut track_recs_added = 0usize;
        let mut seen_rec_track_ids: HashSet<i64> = HashSet::new();
        let mut seen_rec_track_artists: HashSet<i64> = HashSet::new();

        let top_diffused_tracks =
            top_k_from_scores(&ctx.state.graph, ctx.p, ctx.exclude_ids, "track:", Some(track_quota * 4));

        for item in top_diffused_tracks {
            if item.score < ctx.min_rec_score {
                continue;
            }
            if track_recs_added >= track_quota {
                break;
            }
            let Some(("track", tid)) = parse_node_key(&item.node_key) else {
                continue;
            };

            let Ok((ttitle, aname, aid, bpm, pop)) = ctx.conn.query_row(
                "SELECT t.title, a.name, a.id, t.tempo, t.popularity FROM music_tracks t JOIN music_artists a ON a.id = t.artist_id WHERE t.id = ?",
                params![tid],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<f64>>(3)?,
                        r.get::<_, Option<i32>>(4)?,
                    ))
                },
            ) else {
                continue;
            };

            if seen_rec_track_ids.insert(tid) && seen_rec_track_artists.insert(aid) {
                let bpm_str = bpm
                    .filter(|&b| b > 0.0)
                    .map(|b| format!(" • {} BPM", b.round() as i32))
                    .unwrap_or_default();
                let trk_subtitle = if ctx.is_pt {
                    format!("Música de {}{}", aname, bpm_str)
                } else {
                    format!("Track by {}{}", aname, bpm_str)
                };
                let trk_reason = if ctx.is_pt {
                    "Alta ressonância na difusão da trilha sonora e estilo"
                } else {
                    "High resonance in diffusion path and style"
                };

                results.push(json!({
                    "id": item.node_key,
                    "title": ttitle,
                    "type": "track",
                    "subtitle": trk_subtitle,
                    "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                    "top_track": {
                        "title": ttitle,
                        "popularity": pop.unwrap_or(65),
                    },
                    "reason": trk_reason,
                }));

                let art_key = format!("artist:{}", aid);
                if !graph_nodes.iter().any(|n| n.id == art_key) {
                    graph_nodes.push(GraphNodePayload {
                        id: art_key.clone(),
                        label: aname.clone(),
                        node_type: "artist".to_string(),
                        val: 11.5,
                        tags: Vec::new(),
                        desc: String::new(),
                    });
                }
                graph_nodes.push(GraphNodePayload {
                    id: item.node_key.clone(),
                    label: ttitle.clone(),
                    node_type: "track".to_string(),
                    val: 10.5,
                    tags: Vec::new(),
                    desc: String::new(),
                });
                graph_links.push(GraphLinkPayload {
                    source: item.node_key.clone(),
                    target: art_key,
                    label: "performed_by".to_string(),
                    weight: 2.5,
                });
                track_recs_added += 1;
            }
        }

        for (a_id, a_name, a_score, trk, art_reason) in recommended_artist_tracks {
            if track_recs_added >= track_quota {
                break;
            }
            let t_key = format!("track:{}", trk.track_id);
            if ctx.seed_set.contains(&t_key)
                || !seen_rec_track_ids.insert(trk.track_id)
                || !seen_rec_track_artists.insert(a_id)
            {
                continue;
            }

            let bpm_str = trk
                .tempo
                .filter(|&b| b > 0.0)
                .map(|b| format!(" • {} BPM", b.round() as i32))
                .unwrap_or_default();
            let trk_subtitle = if ctx.is_pt {
                format!("Música de {}{}", a_name, bpm_str)
            } else {
                format!("Track by {}{}", a_name, bpm_str)
            };
            let trk_reason = if ctx.is_pt {
                format!("Faixa essencial de {} ({})", a_name, art_reason)
            } else {
                format!("Essential track by {} ({})", a_name, art_reason)
            };

            results.push(json!({
                "id": t_key,
                "title": trk.title,
                "type": "track",
                "subtitle": trk_subtitle,
                "score": ((a_score as f64) * 950.0 * 100.0).round() / 100.0,
                "top_track": {
                    "title": trk.title,
                    "popularity": trk.popularity.unwrap_or(65),
                },
                "reason": trk_reason,
            }));

            let art_key = format!("artist:{}", a_id);
            if !graph_nodes.iter().any(|n| n.id == art_key) {
                graph_nodes.push(GraphNodePayload {
                    id: art_key.clone(),
                    label: a_name.clone(),
                    node_type: "artist".to_string(),
                    val: 11.5,
                    tags: Vec::new(),
                    desc: String::new(),
                });
            }
            graph_nodes.push(GraphNodePayload {
                id: t_key.clone(),
                label: trk.title,
                node_type: "track".to_string(),
                val: 10.5,
                tags: Vec::new(),
                desc: String::new(),
            });
            graph_links.push(GraphLinkPayload {
                source: t_key,
                target: art_key,
                label: "performed_by".to_string(),
                weight: 2.5,
            });
            track_recs_added += 1;
        }
    }
}
