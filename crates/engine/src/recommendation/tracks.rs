//! Dedicated track recommendations with audio feature affinity.

use crate::models::{
    lookup_track_participating_artists, parse_node_key, AppState, TrackRecommendRequest,
    TrackRecommendationItem,
};
use crate::ppr::run_personalized_pagerank_filtered;
use anyhow::Result;
use duckdb::params;
use std::collections::{HashMap, HashSet};

pub fn compute_track_recommendations_sync(
    state: &AppState,
    req: &TrackRecommendRequest,
) -> Result<Vec<TrackRecommendationItem>> {
    if req.seeds.is_empty() {
        return Ok(Vec::new());
    }

    let mut seed_artist_ids: HashSet<i64> = HashSet::new();
    let mut seed_track_ids: HashSet<i64> = HashSet::new();
    let mut seed_audio_features: Vec<(f64, f64, f64, f64)> = Vec::new();

    let conn = state.open_read_conn()?;

    for s in &req.seeds {
        let Some((pfx, id)) = parse_node_key(s) else {
            continue;
        };
        if pfx == "artist" {
            seed_artist_ids.insert(id);
        } else if pfx == "track" {
            seed_track_ids.insert(id);
            if let Some((_, artists)) = lookup_track_participating_artists(&conn, id) {
                for (aid, _) in artists {
                    seed_artist_ids.insert(aid);
                }
            }
            if let Ok((tempo, valence, energy, dance)) = conn.query_row(
                "SELECT tempo, valence, energy, danceability FROM music_tracks WHERE id = ?",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, Option<f64>>(0)?,
                        r.get::<_, Option<f64>>(1)?,
                        r.get::<_, Option<f64>>(2)?,
                        r.get::<_, Option<f64>>(3)?,
                    ))
                },
            ) {
                if let Some(t) = tempo {
                    seed_audio_features.push((
                        t,
                        valence.unwrap_or(0.5),
                        energy.unwrap_or(0.5),
                        dance.unwrap_or(0.5),
                    ));
                }
            }
        }
    }

    if seed_artist_ids.is_empty() {
        return Ok(Vec::new());
    }

    let avg_audio = if !seed_audio_features.is_empty() {
        let n = seed_audio_features.len() as f64;
        Some((
            seed_audio_features.iter().map(|f| f.0).sum::<f64>() / n,
            seed_audio_features.iter().map(|f| f.1).sum::<f64>() / n,
            seed_audio_features.iter().map(|f| f.2).sum::<f64>() / n,
            seed_audio_features.iter().map(|f| f.3).sum::<f64>() / n,
        ))
    } else {
        None
    };

    let seed_keys: Vec<String> = seed_artist_ids
        .iter()
        .map(|aid| format!("artist:{}", aid))
        .collect();
    let seed_refs: Vec<&str> = seed_keys.iter().map(|s| s.as_str()).collect();

    let rust_results = run_personalized_pagerank_filtered(
        &state.graph,
        &seed_refs,
        0.85,
        25,
        1e-6,
        Some("artist:"),
        Some(25),
    )?;

    if rust_results.is_empty() {
        return Ok(Vec::new());
    }

    let mut scored_map: HashMap<i64, f64> = HashMap::new();
    for r in &rust_results {
        if let Some(("artist", aid)) = parse_node_key(&r.node_key) {
            scored_map.insert(aid, r.score as f64);
        }
    }

    if scored_map.is_empty() {
        return Ok(Vec::new());
    }

    let art_ids_str = scored_map
        .keys()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let seed_ids_str = seed_artist_ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let sql_candidates = format!(
        "WITH target_artists AS (
            SELECT id, name FROM music_artists WHERE id IN ({art_ids_str})
        ),
        artist_reasons AS (
            SELECT 
                s.target_artist_id AS artist_id,
                'Last.fm co-listening with ' || ma_src.name || ' (' || CAST(ROUND(s.match_score * 100) AS INT) || '%)' AS reason
            FROM artist_similarities s
            JOIN music_artists ma_src ON ma_src.id = s.source_artist_id
            WHERE s.source_artist_id IN ({seed_ids_str})
              AND s.target_artist_id IN ({art_ids_str})
            UNION ALL
            SELECT 
                t2.artist_id,
                'Shared microgenres with ' || ma_src.name || ': ' || STRING_AGG(k.name, ', ') AS reason
            FROM artist_tags t1
            JOIN music_artists ma_src ON ma_src.id = t1.artist_id
            JOIN keywords k ON k.id = t1.keyword_id
            JOIN artist_tags t2 ON t2.keyword_id = k.id AND t2.artist_id IN ({art_ids_str})
            WHERE t1.artist_id IN ({seed_ids_str})
            GROUP BY t2.artist_id, ma_src.name
        ),
        ranked_tracks AS (
            SELECT 
                t.id,
                t.title,
                t.artist_id,
                ta.name AS artist_name,
                alb.title AS album_name,
                t.popularity,
                t.tempo,
                t.valence,
                t.energy,
                t.danceability,
                COALESCE(ar.reason, 'Musical scene diffusion affinity') AS explanation,
                ROW_NUMBER() OVER (PARTITION BY t.artist_id ORDER BY t.popularity DESC) AS rk
            FROM music_tracks t
            JOIN target_artists ta ON ta.id = t.artist_id
            LEFT JOIN music_albums alb ON alb.id = t.album_id
            LEFT JOIN (
                SELECT artist_id, STRING_AGG(reason, ' • ') AS reason 
                FROM artist_reasons 
                GROUP BY artist_id
            ) ar ON ar.artist_id = t.artist_id
            WHERE t.popularity >= {min_pop}
        )
        SELECT id, title, artist_name, album_name, popularity, tempo, valence, energy, danceability, artist_id, explanation
        FROM ranked_tracks
        WHERE rk <= 2
        ORDER BY popularity DESC
        LIMIT 60;",
        art_ids_str = art_ids_str,
        seed_ids_str = seed_ids_str,
        min_pop = req.min_popularity
    );

    let candidates: Vec<(
        i64,
        String,
        String,
        Option<String>,
        i32,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        i64,
        String,
    )> = {
        let mut stmt = conn.prepare(&sql_candidates)?;
        let mut rows = stmt.query([])?;
        let mut list = Vec::new();
        while let Some(r) = rows.next()? {
            list.push((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
            ));
        }
        list
    };

    let max_graph_score = scored_map
        .values()
        .copied()
        .fold(0.0f64, |a, b| if a > b { a } else { b })
        .max(1e-9);

    let mut recommendations = Vec::new();
    for c in candidates {
        let t_id = c.0;
        let a_id = c.9;
        if seed_track_ids.contains(&t_id) {
            continue;
        }

        let g_score = scored_map.get(&a_id).copied().unwrap_or(0.0);
        let graph_norm = (g_score / max_graph_score).clamp(0.0, 1.0);

        let mut audio_affinity = 0.8;
        if let (Some((avg_t, avg_v, avg_e, avg_d)), Some(ct), Some(cv), Some(ce)) =
            (avg_audio, c.5, c.6, c.7)
        {
            let cd = c.8.unwrap_or(0.5);
            let d_valence = (cv - avg_v).powi(2);
            let d_energy = (ce - avg_e).powi(2);
            let d_dance = (cd - avg_d).powi(2);
            let d_tempo = ((ct - avg_t) / 120.0).powi(2);
            let dist = (0.3 * d_valence + 0.3 * d_energy + 0.2 * d_dance + 0.2 * d_tempo).sqrt();
            audio_affinity = (1.0 - dist).max(0.0);
        }

        let final_affinity =
            (1.0 - req.audio_weight) * graph_norm + req.audio_weight * audio_affinity;
        let affinity_percentage = (final_affinity * 1000.0).round() / 10.0;

        recommendations.push(TrackRecommendationItem {
            track_id: t_id,
            title: c.1,
            artist_name: c.2,
            album_name: c.3,
            popularity: c.4,
            affinity_score: affinity_percentage,
            tempo: c.5,
            valence: c.6,
            energy: c.7,
            danceability: c.8,
            explanation: c.10,
        });
    }

    recommendations.sort_by(|a, b| {
        b.affinity_score
            .partial_cmp(&a.affinity_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    recommendations.truncate(req.limit);

    Ok(recommendations)
}
