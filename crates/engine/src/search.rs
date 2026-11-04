use crate::models::{title_case, AppState, EntityItem, SearchParams};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use duckdb::params;
use serde_json::json;
use std::{collections::HashSet, sync::Arc};

/// Executes entity search synchronously using a dedicated read-only DuckDB connection.
pub fn search_entities_sync(
    state: &AppState,
    params: &SearchParams,
) -> anyhow::Result<Vec<EntityItem>> {
    let q_str = params.q.trim().to_string();
    if q_str.is_empty() {
        return Ok(Vec::new());
    }

    let entity_type = params.entity_type.as_deref().unwrap_or("");
    let limit = params.limit.clamp(1, 30);

    let mut items: Vec<EntityItem> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let conn = state.open_read_conn()?;

    // 1. Artists (if entity_type is empty or 'artist')
    if entity_type.is_empty() || entity_type == "artist" {
        let q_lower = q_str.to_lowercase();
        let q_title = title_case(&q_str);
        let q_upper = q_str.to_uppercase();

        let mut base_variants = vec![q_str.clone(), q_lower.clone(), q_title, q_upper];
        match q_lower.as_str() {
            "ye" | "yeezy" => base_variants.push("Kanye West".to_string()),
            "kesha" => base_variants.push("Ke$ha".to_string()),
            "pink" => base_variants.push("P!nk".to_string()),
            _ => {}
        }

        let mut variants: Vec<String> = Vec::new();
        for bv in base_variants {
            let v_curly = bv.replace('\'', "’");
            let v_hyphen = bv.replace('-', "\u{2010}");
            let v_endash = bv.replace('-', "\u{2013}");
            for cand in [bv, v_curly, v_hyphen, v_endash] {
                if !variants.contains(&cand) {
                    variants.push(cand);
                }
            }
        }

        let q_prefix_curly = q_str.replace('\'', "’").replace('-', "\u{2010}");
        let placeholders = variants.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT id, name, country, listeners 
             FROM music_artists 
             WHERE name IN ({placeholders}) OR name ILIKE ? || '%' OR name ILIKE ? || '%'
             ORDER BY 
                 CASE WHEN LOWER(name) = LOWER(?) OR LOWER(name) = LOWER(?) THEN 1
                      WHEN name NOT LIKE '%;%' THEN 2
                      ELSE 3 END,
                 LENGTH(name) ASC,
                 listeners DESC 
             LIMIT 30;"
        );

        let mut query_params: Vec<&dyn duckdb::ToSql> = Vec::new();
        for v in &variants {
            query_params.push(v);
        }
        query_params.push(&q_str);
        query_params.push(&q_prefix_curly);
        query_params.push(&q_str);
        query_params.push(&q_prefix_curly);

        let norm_artist_key = |s: &str| -> String {
            s.trim()
                .to_lowercase()
                .replace('’', "'")
                .replace('‘', "'")
                .replace(['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}'], "-")
        };
        let q_norm = norm_artist_key(&q_str);

        let mut artist_candidates: Vec<(usize, usize, usize, EntityItem, String)> = Vec::new();
        if let Ok(mut stmt) = conn.prepare(&sql) {
            if let Ok(mut rows) = stmt.query(query_params.as_slice()) {
                while let Ok(Some(r)) = rows.next() {
                    let aid: i64 = r.get(0).unwrap_or_default();
                    let aname: String = r.get(1).unwrap_or_default();
                    let country: Option<String> = r.get(2).ok();
                    let nid = format!("artist:{}", aid);
                    let degree = state.graph.out_edges_by_key(&nid).len();
                    if degree < 5 {
                        continue;
                    }
                    let nkey = norm_artist_key(&aname);
                    let is_alias_exact = ((q_norm == "ye" || q_norm == "yeezy")
                        && nkey == "kanye west")
                        || (q_norm == "kesha" && nkey == "ke$ha")
                        || (q_norm == "pink" && nkey == "p!nk");
                    let exact_prio = if nkey == q_norm || is_alias_exact {
                        0
                    } else if !aname.contains(';') {
                        1
                    } else {
                        2
                    };
                    let sub = country
                        .map(|c| format!("Artist • {}", c))
                        .unwrap_or_else(|| "Artist".to_string());
                    artist_candidates.push((
                        exact_prio,
                        aname.len(),
                        degree,
                        EntityItem {
                            id: nid,
                            name: aname,
                            entity_type: "artist".to_string(),
                            subtitle: Some(sub),
                            extra: None,
                        },
                        nkey,
                    ));
                }
            }
        }

        artist_candidates.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| b.2.cmp(&a.2))
                .then_with(|| a.1.cmp(&b.1))
        });

        let mut seen_artist_names: HashSet<String> = HashSet::new();
        for (_, _, _, item, nkey) in artist_candidates {
            if seen_artist_names.insert(nkey) && seen.insert(item.id.clone()) {
                items.push(item);
                if items.len() >= 6 {
                    break;
                }
            }
        }
    }

    // 2. Games (if entity_type is empty or 'game')
    if entity_type.is_empty() || entity_type == "game" {
        let sql = "
            SELECT id, title, release_year
            FROM games 
            WHERE title = ? OR title ILIKE ? || '%' OR title ILIKE '%' || ? || '%'
            ORDER BY 
                CASE WHEN LOWER(title) = LOWER(?) THEN 1 
                     WHEN LOWER(title) LIKE LOWER(?) || '%' THEN 2 
                     ELSE 3 END,
                positive_reviews DESC NULLS LAST
            LIMIT 6;
        ";
        if let Ok(mut stmt) = conn.prepare(sql) {
            if let Ok(mut rows) = stmt.query(params![&q_str, &q_str, &q_str, &q_str, &q_str]) {
                while let Ok(Some(r)) = rows.next() {
                    let gid: i64 = r.get(0).unwrap_or_default();
                    let gtitle: String = r.get(1).unwrap_or_default();
                    let gyear: Option<i32> = r.get(2).ok();
                    let nid = format!("game:{}", gid);
                    if seen.insert(nid.clone()) {
                        let year_str = gyear.map(|y| format!(" ({})", y)).unwrap_or_default();
                        items.push(EntityItem {
                            id: nid,
                            name: gtitle,
                            entity_type: "game".to_string(),
                            subtitle: Some(format!("Game{}", year_str)),
                            extra: None,
                        });
                    }
                }
            }
        }
    }

    // 3. Movies (if entity_type is empty or 'movie')
    if entity_type.is_empty() || entity_type == "movie" {
        let sql = "
            SELECT id, title, release_year, vote_average
            FROM movies
            WHERE title = ? OR title ILIKE ? || '%' OR title ILIKE '%' || ? || '%'
            ORDER BY
                CASE WHEN LOWER(title) = LOWER(?) THEN 1
                     WHEN LOWER(title) LIKE LOWER(?) || '%' THEN 2
                     ELSE 3 END,
                vote_count DESC NULLS LAST
            LIMIT 6;
        ";
        if let Ok(mut stmt) = conn.prepare(sql) {
            if let Ok(mut rows) = stmt.query(params![&q_str, &q_str, &q_str, &q_str, &q_str]) {
                while let Ok(Some(r)) = rows.next() {
                    let mid: i64 = r.get(0).unwrap_or_default();
                    let mtitle: String = r.get(1).unwrap_or_default();
                    let myear: Option<i32> = r.get(2).ok();
                    let mvote: Option<f64> = r.get(3).ok();
                    let nid = format!("movie:{}", mid);
                    if seen.insert(nid.clone()) {
                        let year_str = myear.map(|y| format!(" ({})", y)).unwrap_or_default();
                        let score_str = mvote
                            .map(|v| format!(" • Rating: {:.1}", v))
                            .unwrap_or_default();
                        items.push(EntityItem {
                            id: nid,
                            name: mtitle,
                            entity_type: "movie".to_string(),
                            subtitle: Some(format!("Movie{}{}", year_str, score_str)),
                            extra: None,
                        });
                    }
                }
            }
        }
    }

    // 4. TV Series (if entity_type is empty or 'series')
    if entity_type.is_empty() || entity_type == "series" {
        let sql = "
            SELECT id, title, start_year, end_year, vote_average, series_type
            FROM tv_series
            WHERE title = ? OR title ILIKE ? || '%' OR title ILIKE '%' || ? || '%'
            ORDER BY
                CASE WHEN LOWER(title) = LOWER(?) THEN 1
                     WHEN LOWER(title) LIKE LOWER(?) || '%' THEN 2
                     ELSE 3 END,
                vote_count DESC NULLS LAST
            LIMIT 6;
        ";
        if let Ok(mut stmt) = conn.prepare(sql) {
            if let Ok(mut rows) = stmt.query(params![&q_str, &q_str, &q_str, &q_str, &q_str]) {
                while let Ok(Some(r)) = rows.next() {
                    let sid: i64 = r.get(0).unwrap_or_default();
                    let stitle: String = r.get(1).unwrap_or_default();
                    let sstart: Option<i32> = r.get(2).ok();
                    let send: Option<i32> = r.get(3).ok();
                    let svote: Option<f64> = r.get(4).ok();
                    let stype: String = r.get(5).unwrap_or_else(|_| "tvSeries".to_string());
                    let nid = format!("series:{}", sid);
                    if seen.insert(nid.clone()) {
                        let years_str = match (sstart, send) {
                            (Some(s), Some(e)) if s != e => format!(" ({}-{})", s, e),
                            (Some(s), _) => format!(" ({})", s),
                            _ => String::new(),
                        };
                        let type_label = if stype == "tvMiniSeries" { "Mini-Series" } else { "Series" };
                        let score_str = svote
                            .map(|v| format!(" • Rating: {:.1}", v))
                            .unwrap_or_default();
                        items.push(EntityItem {
                            id: nid,
                            name: stitle,
                            entity_type: "series".to_string(),
                            subtitle: Some(format!("{}{}{}", type_label, years_str, score_str)),
                            extra: None,
                        });
                    }
                }
            }
        }
    }

    // 5. Books (if entity_type is 'book')
    if entity_type == "book" {
        let sql = "
            SELECT id, title, first_publish_year
            FROM books
            WHERE title = ? OR title ILIKE ? || '%' OR title ILIKE '%' || ? || '%'
            ORDER BY
                CASE WHEN LOWER(title) = LOWER(?) THEN 1
                     WHEN LOWER(title) LIKE LOWER(?) || '%' THEN 2
                     ELSE 3 END
            LIMIT 6;
        ";
        if let Ok(mut stmt) = conn.prepare(sql) {
            if let Ok(mut rows) = stmt.query(params![&q_str, &q_str, &q_str, &q_str, &q_str]) {
                while let Ok(Some(r)) = rows.next() {
                    let bid: i64 = r.get(0).unwrap_or_default();
                    let btitle: String = r.get(1).unwrap_or_default();
                    let byear: Option<i32> = r.get(2).ok();
                    let nid = format!("book:{}", bid);
                    if seen.insert(nid.clone()) {
                        let year_str = byear.map(|y| format!(" ({})", y)).unwrap_or_default();
                        items.push(EntityItem {
                            id: nid,
                            name: btitle,
                            entity_type: "book".to_string(),
                            subtitle: Some(format!("Book{}", year_str)),
                            extra: None,
                        });
                    }
                }
            }
        }
    }

    // 6. Music tracks (if entity_type is empty or 'track')
    if entity_type.is_empty() || entity_type == "track" {
        let needed = (limit.saturating_sub(items.len()) * 3).max(10) as i64;
        let sql = "
            SELECT t.id, t.title, ma.name, t.popularity, t.tempo, t.valence, t.energy, alb.title
            FROM music_tracks t
            JOIN music_artists ma ON ma.id = t.artist_id
            LEFT JOIN music_albums alb ON alb.id = t.album_id
            WHERE t.title = ? OR t.title ILIKE ? || '%' OR t.title ILIKE '%' || ? || '%'
            ORDER BY
                CASE WHEN LOWER(t.title) = LOWER(?) THEN 1
                     WHEN LOWER(t.title) LIKE LOWER(?) || '%' THEN 2
                     ELSE 3 END,
                t.popularity DESC
            LIMIT ?;
        ";
        if let Ok(mut stmt) = conn.prepare(sql) {
            if let Ok(mut rows) =
                stmt.query(params![&q_str, &q_str, &q_str, &q_str, &q_str, needed])
            {
                let mut seen_track_keys: HashSet<(String, String)> = HashSet::new();
                while let Ok(Some(r)) = rows.next() {
                    let tid: i64 = r.get(0).unwrap_or_default();
                    let ttitle: String = r.get(1).unwrap_or_default();
                    let taname: String = r.get(2).unwrap_or_default();
                    let tpop: Option<i32> = r.get(3).ok();
                    let bpm: Option<f64> = r.get(4).ok();
                    let albtitle: Option<String> = r.get(7).ok();

                    let key = (ttitle.trim().to_lowercase(), taname.trim().to_lowercase());
                    if !seen_track_keys.insert(key) {
                        continue;
                    }
                    let nid = format!("track:{}", tid);
                    if seen.insert(nid.clone()) {
                        let alb_str = albtitle.map(|a| format!(" • {}", a)).unwrap_or_default();
                        let vibe = bpm
                            .filter(|&b| b > 0.0)
                            .map(|b| format!(" ({} BPM)", b.round() as i32))
                            .unwrap_or_default();
                        items.push(EntityItem {
                            id: nid,
                            name: ttitle,
                            entity_type: "track".to_string(),
                            subtitle: Some(format!("{}{}{}", taname, alb_str, vibe)),
                            extra: Some(json!({
                                "popularity": tpop,
                                "artist_name": taname,
                            })),
                        });
                    }
                    if items.len() >= limit {
                        break;
                    }
                }
            }
        }
    }

    items.truncate(limit);
    Ok(items)
}

/// Axum HTTP handler that offloads search to the blocking task worker pool.
pub async fn api_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<EntityItem>>, (StatusCode, String)> {
    tokio::task::spawn_blocking(move || {
        search_entities_sync(&state, &params)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Task join error: {}", e)))?
}
