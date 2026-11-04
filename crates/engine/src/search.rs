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

    let raw_entity_type = params.entity_type.as_deref().unwrap_or("");
    let entity_type = if raw_entity_type.is_empty() && q_str.starts_with('#') {
        "keyword"
    } else {
        raw_entity_type
    };
    let limit = params.limit.clamp(1, 30);

    let mut items: Vec<EntityItem> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

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
        if let Ok(conn) = state.db_conn.lock() {
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

    // 2. Games (if entity_type is empty or 'game') — resolved directly from in-memory CatalogCache
    if entity_type.is_empty() || entity_type == "game" {
        let q_lower = q_str.to_lowercase();
        let q_no_the = q_lower.strip_prefix("the ").unwrap_or(&q_lower);
        let mut game_candidates: Vec<(u8, u8, i64, usize, i64, &crate::models::GameMeta)> = Vec::new();
        for (&gid, meta) in &state.catalog.games {
            let t_lower = meta.title.to_lowercase();
            let t_no_the = t_lower.strip_prefix("the ").unwrap_or(&t_lower);
            let match_tier = if t_lower == q_lower || t_no_the == q_no_the {
                0
            } else if t_lower.ends_with(&format!(": {}", q_lower))
                || t_lower.starts_with(&format!("{} ", q_lower))
                || t_no_the.starts_with(&format!("{} ", q_no_the))
                || t_lower.starts_with(&q_lower)
                || t_no_the.starts_with(q_no_the)
            {
                1
            } else if t_lower.contains(&q_lower) {
                2
            } else {
                continue;
            };
            let edition_penalty = if t_lower.contains("special edition")
                || t_lower.contains(" vr")
                || t_lower.contains("remaster")
                || t_lower.contains("deluxe")
            {
                1
            } else {
                0
            };
            let pos_rev = meta.positive_reviews.unwrap_or(0);
            game_candidates.push((
                match_tier,
                edition_penalty,
                -pos_rev,
                meta.title.len(),
                gid,
                meta,
            ));
        }
        game_candidates.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.3.cmp(&b.3))
        });

        let mut seen_game_titles: HashSet<String> = HashSet::new();
        let mut games_added = 0usize;
        let max_games = if entity_type == "game" { limit } else { 6 };
        for (_, _, _, _, gid, meta) in game_candidates {
            let norm_gtitle = crate::recommendation::context::normalize_game_title(&meta.title);
            if !seen_game_titles.insert(norm_gtitle) {
                continue;
            }
            let nid = format!("game:{}", gid);
            if seen.insert(nid.clone()) {
                let year_str = meta
                    .release_year
                    .map(|y| format!(" ({})", y))
                    .unwrap_or_default();
                items.push(EntityItem {
                    id: nid,
                    name: meta.title.clone(),
                    entity_type: "game".to_string(),
                    subtitle: Some(format!("Game{}", year_str)),
                    extra: None,
                });
                games_added += 1;
                if games_added >= max_games {
                    break;
                }
            }
        }
    }

    // 3. Movies (if entity_type is empty or 'movie') — resolved from in-memory CatalogCache
    if entity_type.is_empty() || entity_type == "movie" {
        let q_lower = q_str.to_lowercase();
        let q_no_the = q_lower.strip_prefix("the ").unwrap_or(&q_lower);
        let mut movie_candidates: Vec<(u8, usize, i64, &crate::models::MovieMeta)> = Vec::new();
        for (&mid, meta) in &state.catalog.movies {
            let t_lower = meta.title.to_lowercase();
            let t_no_the = t_lower.strip_prefix("the ").unwrap_or(&t_lower);
            let match_tier = if t_lower == q_lower || t_no_the == q_no_the {
                0
            } else if t_lower.starts_with(&format!("{} ", q_lower))
                || t_no_the.starts_with(&format!("{} ", q_no_the))
                || t_lower.starts_with(&q_lower)
                || t_no_the.starts_with(q_no_the)
            {
                1
            } else if t_lower.contains(&q_lower) {
                2
            } else {
                continue;
            };
            let deg = state.graph.out_edges_by_key(&format!("movie:{}", mid)).len();
            movie_candidates.push((match_tier, deg, mid, meta));
        }
        movie_candidates.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.3.title.len().cmp(&b.3.title.len()))
        });
        let max_movies = if entity_type == "movie" { limit } else { 6 };
        for (_, _, mid, meta) in movie_candidates.into_iter().take(max_movies) {
            let nid = format!("movie:{}", mid);
            if seen.insert(nid.clone()) {
                let year_str = meta
                    .release_year
                    .map(|y| format!(" ({})", y))
                    .unwrap_or_default();
                let score_str = meta
                    .vote_average
                    .map(|v| format!(" • Rating: {:.1}", v))
                    .unwrap_or_default();
                items.push(EntityItem {
                    id: nid,
                    name: meta.title.clone(),
                    entity_type: "movie".to_string(),
                    subtitle: Some(format!("Movie{}{}", year_str, score_str)),
                    extra: None,
                });
            }
        }
    }

    // 4. TV Series (if entity_type is empty or 'series') — resolved from in-memory CatalogCache
    if entity_type.is_empty() || entity_type == "series" {
        let q_lower = q_str.to_lowercase();
        let q_no_the = q_lower.strip_prefix("the ").unwrap_or(&q_lower);
        let mut series_candidates: Vec<(u8, usize, i64, &crate::models::SeriesMeta)> = Vec::new();
        for (&sid, meta) in &state.catalog.series {
            let t_lower = meta.title.to_lowercase();
            let t_no_the = t_lower.strip_prefix("the ").unwrap_or(&t_lower);
            let match_tier = if t_lower == q_lower || t_no_the == q_no_the {
                0
            } else if t_lower.starts_with(&format!("{} ", q_lower))
                || t_no_the.starts_with(&format!("{} ", q_no_the))
                || t_lower.starts_with(&q_lower)
                || t_no_the.starts_with(q_no_the)
            {
                1
            } else if t_lower.contains(&q_lower) {
                2
            } else {
                continue;
            };
            let deg = state.graph.out_edges_by_key(&format!("series:{}", sid)).len();
            series_candidates.push((match_tier, deg, sid, meta));
        }
        series_candidates.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.3.title.len().cmp(&b.3.title.len()))
        });
        let max_series = if entity_type == "series" { limit } else { 6 };
        for (_, _, sid, meta) in series_candidates.into_iter().take(max_series) {
            let nid = format!("series:{}", sid);
            if seen.insert(nid.clone()) {
                let years_str = match (meta.start_year, meta.end_year) {
                    (Some(s), Some(e)) if s != e => format!(" ({}-{})", s, e),
                    (Some(s), _) => format!(" ({})", s),
                    _ => String::new(),
                };
                let type_label = if meta.series_type == "tvMiniSeries" {
                    "Mini-Series"
                } else {
                    "Series"
                };
                let score_str = meta
                    .vote_average
                    .map(|v| format!(" • Rating: {:.1}", v))
                    .unwrap_or_default();
                items.push(EntityItem {
                    id: nid,
                    name: meta.title.clone(),
                    entity_type: "series".to_string(),
                    subtitle: Some(format!("{}{}{}", type_label, years_str, score_str)),
                    extra: None,
                });
            }
        }
    }

    // 5. Books (if entity_type is 'book') — resolved from in-memory CatalogCache
    if entity_type == "book" {
        let q_lower = q_str.to_lowercase();
        let mut book_candidates: Vec<(u8, usize, i64, &crate::models::BookMeta)> = Vec::new();
        for (&bid, meta) in &state.catalog.books {
            let t_lower = meta.title.to_lowercase();
            let match_tier = if t_lower == q_lower {
                0
            } else if t_lower.starts_with(&q_lower) {
                1
            } else if t_lower.contains(&q_lower) {
                2
            } else {
                continue;
            };
            book_candidates.push((match_tier, meta.title.len(), bid, meta));
        }
        book_candidates.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for (_, _, bid, meta) in book_candidates.into_iter().take(limit) {
            let nid = format!("book:{}", bid);
            if seen.insert(nid.clone()) {
                let year_str = meta
                    .first_publish_year
                    .map(|y| format!(" ({})", y))
                    .unwrap_or_default();
                items.push(EntityItem {
                    id: nid,
                    name: meta.title.clone(),
                    entity_type: "book".to_string(),
                    subtitle: Some(format!("Book{}", year_str)),
                    extra: None,
                });
            }
        }
    }

    // 5b. Keywords / Tags (if entity_type is 'keyword' or query starts with '#') — resolved from CatalogCache
    if entity_type == "keyword" || q_str.starts_with('#') {
        let clean_tag = q_str.trim_start_matches('#').trim().to_lowercase();
        if !clean_tag.is_empty() {
            let mut kw_candidates: Vec<(u8, usize, i64, &String)> = Vec::new();
            for (&kid, kname) in &state.catalog.keywords {
                let k_lower = kname.to_lowercase();
                let match_tier = if k_lower == clean_tag {
                    0
                } else if k_lower.starts_with(&clean_tag) {
                    1
                } else if entity_type == "keyword" && k_lower.contains(&clean_tag) {
                    2
                } else {
                    continue;
                };
                kw_candidates.push((match_tier, kname.len(), kid, kname));
            }
            kw_candidates.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            let max_kw = if entity_type == "keyword" { limit } else { 6 };
            for (_, _, kid, kname) in kw_candidates.into_iter().take(max_kw) {
                let nid = format!("keyword:{}", kid);
                if seen.insert(nid.clone()) {
                    items.push(EntityItem {
                        id: nid,
                        name: format!("#{}", kname),
                        entity_type: "keyword".to_string(),
                        subtitle: Some("Tag".to_string()),
                        extra: None,
                    });
                }
            }
        }
    }

    // 6. Music tracks (only when entity_type is explicitly 'track' or when no primary entities matched)
    if entity_type == "track" || (entity_type.is_empty() && items.is_empty()) {
        let needed = (limit * 3).max(15) as i64;
        let sql = "
            WITH matched_tracks AS (
                SELECT id, title, artist_id, album_id, popularity, tempo
                FROM music_tracks
                WHERE title = ? OR title ILIKE ? || '%'
                ORDER BY
                    CASE WHEN LOWER(title) = LOWER(?) THEN 1 ELSE 2 END,
                    popularity DESC NULLS LAST
                LIMIT ?
            )
            SELECT t.id, t.title, ma.name, t.popularity, t.tempo, alb.title
            FROM matched_tracks t
            JOIN music_artists ma ON ma.id = t.artist_id
            LEFT JOIN music_albums alb ON alb.id = t.album_id
            ORDER BY
                CASE WHEN LOWER(t.title) = LOWER(?) THEN 1 ELSE 2 END,
                t.popularity DESC NULLS LAST;
        ";
        if let Ok(conn) = state.db_conn.lock() {
            if let Ok(mut stmt) = conn.prepare(sql) {
                if let Ok(mut rows) =
                    stmt.query(params![&q_str, &q_str, &q_str, needed, &q_str])
                {
                    let mut seen_track_keys: HashSet<(String, String)> = HashSet::new();
                    while let Ok(Some(r)) = rows.next() {
                        let tid: i64 = r.get(0).unwrap_or_default();
                        let ttitle: String = r.get(1).unwrap_or_default();
                        let taname: String = r.get(2).unwrap_or_default();
                        let tpop: Option<i32> = r.get(3).ok();
                        let bpm: Option<f64> = r.get(4).ok();
                        let albtitle: Option<String> = r.get(5).ok();

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
                    }
                }
            }
        }
    }

    // When searching across all types, sort by match quality and graph connectivity so canonical works surface first.
    if entity_type.is_empty() {
        let q_lower = q_str.trim_start_matches('#').to_lowercase();
        let q_no_the = q_lower.strip_prefix("the ").unwrap_or(&q_lower);
        items.sort_by(|a, b| {
            let a_name = a.name.trim_start_matches('#').to_lowercase();
            let b_name = b.name.trim_start_matches('#').to_lowercase();
            let tier = |name: &str| -> u8 {
                let n_no_the = name.strip_prefix("the ").unwrap_or(name);
                if name == q_lower
                    || n_no_the == q_no_the
                    || name.ends_with(&format!(": {}", q_lower))
                {
                    0
                } else if name.starts_with(&format!("{} ", q_lower))
                    || name.starts_with(&format!("{}:", q_lower))
                    || n_no_the.starts_with(&format!("{} ", q_no_the))
                    || n_no_the.starts_with(&format!("{}:", q_no_the))
                {
                    1
                } else if name.starts_with(&q_lower) || n_no_the.starts_with(q_no_the) {
                    2
                } else {
                    3
                }
            };
            let deg_a = state.graph.out_edges_by_key(&a.id).len();
            let deg_b = state.graph.out_edges_by_key(&b.id).len();
            tier(&a_name)
                .cmp(&tier(&b_name))
                .then_with(|| deg_b.cmp(&deg_a))
                .then_with(|| a.name.len().cmp(&b.name.len()))
        });
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
