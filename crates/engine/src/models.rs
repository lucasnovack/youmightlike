use crate::graph::GraphCSR;
use anyhow::Result;
use duckdb::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Mutex,
};

pub const MAX_CACHE_SIZE: usize = 128;

#[derive(Debug, Clone)]
pub struct MovieMeta {
    pub title: String,
    pub release_year: Option<i32>,
    pub vote_average: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct SeriesMeta {
    pub title: String,
    pub series_type: String,
    pub start_year: Option<i32>,
    pub end_year: Option<i32>,
    pub vote_average: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct GameMeta {
    pub title: String,
    pub release_year: Option<i32>,
    pub positive_reviews: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct BookMeta {
    pub title: String,
    pub first_publish_year: Option<i32>,
}

/// Compact dimensional catalog in RAM for O(1) resolution without disk I/O during subgraph construction.
pub struct CatalogCache {
    pub keywords: HashMap<i64, String>,
    pub people: HashMap<i64, String>,
    pub developers: HashMap<i64, String>,
    pub authors: HashMap<i64, String>,
    pub movies: HashMap<i64, MovieMeta>,
    pub series: HashMap<i64, SeriesMeta>,
    pub games: HashMap<i64, GameMeta>,
    pub books: HashMap<i64, BookMeta>,
}

impl CatalogCache {
    pub fn load_from_duckdb(conn: &Connection) -> Result<Self> {
        let mut keywords = HashMap::with_capacity(50_000);
        {
            let mut stmt = conn.prepare("SELECT id, name FROM keywords")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                keywords.insert(r.get(0)?, r.get(1)?);
            }
        }

        let mut people = HashMap::with_capacity(40_000);
        {
            let mut stmt = conn.prepare("SELECT id, name FROM people")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                people.insert(r.get(0)?, r.get(1)?);
            }
        }

        let mut developers = HashMap::with_capacity(10_000);
        {
            let mut stmt = conn.prepare("SELECT id, name FROM game_developers")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                developers.insert(r.get(0)?, r.get(1)?);
            }
        }

        let mut authors = HashMap::with_capacity(32_000);
        {
            let mut stmt = conn.prepare("SELECT id, name FROM book_authors")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                authors.insert(r.get(0)?, r.get(1)?);
            }
        }

        let mut movies = HashMap::with_capacity(36_000);
        {
            let mut stmt =
                conn.prepare("SELECT id, title, release_year, vote_average FROM movies")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                movies.insert(
                    r.get(0)?,
                    MovieMeta {
                        title: r.get(1)?,
                        release_year: r.get(2).ok(),
                        vote_average: r.get(3).ok(),
                    },
                );
            }
        }

        let mut series = HashMap::with_capacity(10_000);
        {
            let mut stmt = conn.prepare(
                "SELECT id, title, series_type, start_year, end_year, vote_average FROM tv_series",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                series.insert(
                    r.get(0)?,
                    SeriesMeta {
                        title: r.get(1)?,
                        series_type: r.get(2).unwrap_or_else(|_| "tvSeries".to_string()),
                        start_year: r.get(3).ok(),
                        end_year: r.get(4).ok(),
                        vote_average: r.get(5).ok(),
                    },
                );
            }
        }

        let mut games = HashMap::with_capacity(22_000);
        {
            let mut stmt =
                conn.prepare("SELECT id, title, release_year, positive_reviews FROM games")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                games.insert(
                    r.get(0)?,
                    GameMeta {
                        title: r.get(1)?,
                        release_year: r.get(2).ok(),
                        positive_reviews: r.get(3).ok(),
                    },
                );
            }
        }

        let mut books = HashMap::with_capacity(40_000);
        {
            let mut stmt = conn.prepare("SELECT id, title, first_publish_year FROM books")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                books.insert(
                    r.get(0)?,
                    BookMeta {
                        title: r.get(1)?,
                        first_publish_year: r.get(2).ok(),
                    },
                );
            }
        }

        Ok(Self {
            keywords,
            people,
            developers,
            authors,
            movies,
            series,
            games,
            books,
        })
    }
}

pub struct ResponseCache {
    pub map: HashMap<String, RecommendationResponse>,
    pub order: VecDeque<String>,
}

impl ResponseCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::with_capacity(MAX_CACHE_SIZE),
            order: VecDeque::with_capacity(MAX_CACHE_SIZE),
        }
    }

    pub fn get(&self, key: &str) -> Option<RecommendationResponse> {
        self.map.get(key).cloned()
    }

    pub fn insert(&mut self, key: String, val: RecommendationResponse) {
        if !self.map.contains_key(&key) {
            if self.order.len() >= MAX_CACHE_SIZE {
                if let Some(oldest) = self.order.pop_front() {
                    self.map.remove(&oldest);
                }
            }
            self.order.push_back(key.clone());
        }
        self.map.insert(key, val);
    }
}

pub struct AppState {
    pub db_path: String,
    pub graph: GraphCSR,
    pub catalog: CatalogCache,
    pub db_conn: Mutex<Connection>,
    pub cache: Mutex<ResponseCache>,
}

impl AppState {
    /// Opens a new read-only DuckDB connection dedicated to the current worker thread,
    /// eliminating lock contention between parallel searches and recommendations.
    pub fn open_read_conn(&self) -> Result<Connection> {
        let conn = Connection::open_with_flags(
            &self.db_path,
            duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?,
        )?;
        Ok(conn)
    }
}

// ---------------------------------------------------------------------------
// API Data Models (Compatible with web frontend and legacy API contracts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityItem {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub subtitle: Option<String>,
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchParams {
    pub q: String,
    pub entity_type: Option<String>,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

pub fn default_search_limit() -> usize {
    10
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecommendationRequest {
    pub seeds: Vec<String>,
    pub target_type: Option<String>,
    #[serde(default = "default_rec_limit")]
    pub limit: usize,
    #[serde(default = "default_lang")]
    pub lang: String,
}

pub fn default_rec_limit() -> usize {
    12
}

pub fn default_lang() -> String {
    "pt".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNodePayload {
    pub id: String,
    pub label: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub val: f32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub desc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphLinkPayload {
    pub source: String,
    pub target: String,
    pub label: String,
    pub weight: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPayload {
    pub nodes: Vec<GraphNodePayload>,
    pub links: Vec<GraphLinkPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationResponse {
    pub seeds: Vec<EntityItem>,
    pub results: Vec<Value>,
    pub graph: GraphPayload,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackRecommendRequest {
    pub seeds: Vec<String>,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    #[serde(default = "default_min_pop")]
    pub min_popularity: i32,
    #[serde(default = "default_audio_weight")]
    pub audio_weight: f64,
}

pub fn default_min_pop() -> i32 {
    30
}

pub fn default_audio_weight() -> f64 {
    0.4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackRecommendationItem {
    pub track_id: i64,
    pub title: String,
    pub artist_name: String,
    pub album_name: Option<String>,
    pub popularity: i32,
    pub affinity_score: f64,
    pub tempo: Option<f64>,
    pub valence: Option<f64>,
    pub energy: Option<f64>,
    pub danceability: Option<f64>,
    pub explanation: String,
}

// ---------------------------------------------------------------------------
// Shared Topology and Metadata Helpers
// ---------------------------------------------------------------------------

pub fn is_tag_relation(rel: &str) -> bool {
    matches!(rel, "has_tag" | "has_keyword" | "has_subject")
}

pub fn is_soundtrack_relation(rel: &str) -> bool {
    matches!(
        rel,
        "soundtrack_featured" | "soundtrack_artist" | "soundtrack_track"
    )
}

pub fn parse_node_key(key: &str) -> Option<(&str, i64)> {
    let (pfx, raw) = key.split_once(':')?;
    let id = raw.parse::<i64>().ok()?;
    Some((pfx, id))
}

pub fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
    }
}

pub fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(capitalize_first)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn lookup_artist_meta(conn: &Connection, aid: i64) -> Option<(String, Option<String>)> {
    conn.query_row(
        "SELECT name, country FROM music_artists WHERE id = ?",
        params![aid],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
    )
    .ok()
}

#[derive(Debug, Clone)]
pub struct TopTrackRow {
    pub track_id: i64,
    pub title: String,
    pub album_title: Option<String>,
    pub popularity: Option<i32>,
    pub tempo: Option<f64>,
    pub valence: Option<f64>,
    pub energy: Option<f64>,
}

pub fn lookup_top_track_full(conn: &Connection, aid: i64) -> Option<TopTrackRow> {
    let mut stmt = conn
        .prepare(
            "SELECT id, title, popularity, tempo, valence, energy, album_id
             FROM music_tracks
             WHERE artist_id = ?
             ORDER BY popularity DESC NULLS LAST
             LIMIT 1",
        )
        .ok()?;

    let row = stmt
        .query_row(params![aid], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i32>>(2)?,
                r.get::<_, Option<f64>>(3)?,
                r.get::<_, Option<f64>>(4)?,
                r.get::<_, Option<f64>>(5)?,
                r.get::<_, Option<i64>>(6)?,
            ))
        })
        .ok()?;

    let alb_title = row.6.and_then(|alb_id| {
        conn.query_row(
            "SELECT title FROM music_albums WHERE id = ?",
            params![alb_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
    });

    Some(TopTrackRow {
        track_id: row.0,
        title: row.1,
        album_title: alb_title,
        popularity: row.2,
        tempo: row.3,
        valence: row.4,
        energy: row.5,
    })
}

pub fn lookup_seed_top_tracks(conn: &Connection, aid: i64, limit: usize) -> Vec<TopTrackRow> {
    let mut stmt = match conn.prepare(
        "SELECT id, title, popularity, tempo, valence, energy, album_id
         FROM music_tracks
         WHERE artist_id = ?
         ORDER BY popularity DESC NULLS LAST
         LIMIT ?",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows: Vec<(i64, String, Option<i32>, Option<f64>, Option<f64>, Option<f64>, Option<i64>)> = match stmt
        .query_map(params![aid, limit as i64], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        }) {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(_) => return Vec::new(),
    };

    let mut album_map: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    let album_ids: Vec<i64> = rows.iter().filter_map(|r| r.6).collect();
    if !album_ids.is_empty() {
        let placeholders = album_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let q = format!("SELECT id, title FROM music_albums WHERE id IN ({})", placeholders);
        if let Ok(mut a_stmt) = conn.prepare(&q) {
            let params_vec: Vec<&dyn duckdb::ToSql> = album_ids.iter().map(|id| id as &dyn duckdb::ToSql).collect();
            if let Ok(a_rows) = a_stmt.query_map(&params_vec[..], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
            }) {
                for r in a_rows.flatten() {
                    if let (id, Some(title)) = r {
                        album_map.insert(id, title);
                    }
                }
            }
        }
    }

    rows.into_iter()
        .map(|r| {
            let alb = r.6.and_then(|id| album_map.get(&id).cloned());
            TopTrackRow {
                track_id: r.0,
                title: r.1,
                album_title: alb,
                popularity: r.2,
                tempo: r.3,
                valence: r.4,
                energy: r.5,
            }
        })
        .collect()
}

pub fn resolve_artist_track_and_collab(
    conn: &Connection,
    aid: i64,
    seed_album_titles: &std::collections::HashMap<String, (i64, String)>,
) -> Option<(TopTrackRow, Option<(i64, String)>)> {
    let mut stmt = conn
        .prepare(
            "SELECT id, title, popularity, tempo, valence, energy, album_id
             FROM music_tracks
             WHERE artist_id = ?
             ORDER BY popularity DESC NULLS LAST
             LIMIT 5",
        )
        .ok()?;

    let rows: Vec<(i64, String, Option<i32>, Option<f64>, Option<f64>, Option<f64>, Option<i64>)> = stmt
        .query_map(params![aid], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .ok()?
        .filter_map(|r| r.ok())
        .collect();

    if rows.is_empty() {
        return None;
    }

    let mut album_map: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    let album_ids: Vec<i64> = rows.iter().filter_map(|r| r.6).collect();
    if !album_ids.is_empty() {
        let placeholders = album_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let q = format!("SELECT id, title FROM music_albums WHERE id IN ({})", placeholders);
        if let Ok(mut a_stmt) = conn.prepare(&q) {
            let params_vec: Vec<&dyn duckdb::ToSql> = album_ids.iter().map(|id| id as &dyn duckdb::ToSql).collect();
            if let Ok(a_rows) = a_stmt.query_map(&params_vec[..], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
            }) {
                for r in a_rows.flatten() {
                    if let (id, Some(title)) = r {
                        album_map.insert(id, title);
                    }
                }
            }
        }
    }

    for r in &rows {
        if let Some(alb_id) = r.6 {
            if let Some(alb_title) = album_map.get(&alb_id) {
                let norm = alb_title.trim().to_lowercase();
                if norm.len() >= 3 && norm != "single" && norm != "ep" && norm != "greatest hits" {
                    if let Some((seed_aid, seed_name)) = seed_album_titles.get(&norm) {
                        if *seed_aid != aid {
                            return Some((
                                TopTrackRow {
                                    track_id: r.0,
                                    title: r.1.clone(),
                                    album_title: Some(alb_title.clone()),
                                    popularity: r.2,
                                    tempo: r.3,
                                    valence: r.4,
                                    energy: r.5,
                                },
                                Some((*seed_aid, seed_name.clone())),
                            ));
                        }
                    }
                }
            }
        }
    }

    let first = &rows[0];
    let alb_title = first.6.and_then(|id| album_map.get(&id).cloned());
    Some((
        TopTrackRow {
            track_id: first.0,
            title: first.1.clone(),
            album_title: alb_title,
            popularity: first.2,
            tempo: first.3,
            valence: first.4,
            energy: first.5,
        },
        None,
    ))
}

fn extract_featured_artist_names(title: &str) -> Option<Vec<String>> {
    let lower = title.to_lowercase();
    let (start_idx, has_bracket) = if let Some(idx) = lower.find("(feat. ") {
        (idx + 7, true)
    } else if let Some(idx) = lower.find("(ft. ") {
        (idx + 5, true)
    } else if let Some(idx) = lower.find("[feat. ") {
        (idx + 7, true)
    } else if let Some(idx) = lower.find("[ft. ") {
        (idx + 5, true)
    } else if let Some(idx) = lower.find(" feat. ") {
        (idx + 7, false)
    } else if let Some(idx) = lower.find(" ft. ") {
        (idx + 5, false)
    } else {
        return None;
    };

    let rem = &title[start_idx..];
    let content = if has_bracket {
        let end_idx = rem.find(')').or_else(|| rem.find(']'))?;
        &rem[..end_idx]
    } else if let Some(end_idx) = rem.find(" - ") {
        &rem[..end_idx]
    } else {
        rem
    };

    let parts: Vec<String> = content
        .split(|c| c == ',' || c == '&')
        .flat_map(|s| s.split(" and "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

pub fn lookup_track_participating_artists(
    conn: &Connection,
    track_id: i64,
) -> Option<(String, Vec<(i64, String)>)> {
    let mut stmt = conn
        .prepare(
            "SELECT t.title, t.artist_id, ma.name, t.spotify_id, t.album_id
             FROM music_tracks t
             JOIN music_artists ma ON ma.id = t.artist_id
             WHERE t.id = ?",
        )
        .ok()?;

    let (title, primary_aid, primary_name, spotify_id, album_id) = stmt
        .query_row(params![track_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })
        .ok()?;

    let mut artists: Vec<(i64, String)> = Vec::new();
    let mut seen_aids: HashSet<i64> = HashSet::new();

    artists.push((primary_aid, primary_name));
    seen_aids.insert(primary_aid);

    // Method 1: If spotify_id exists and raw parquet is available, query parquet for all co-artists
    if let Some(ref sid) = spotify_id {
        if std::path::Path::new("data/raw/music/spotify_56m_tracks.parquet").exists() {
            let parquet_sql = "
                SELECT DISTINCT ma.id, ma.name
                FROM 'data/raw/music/spotify_56m_tracks.parquet' p
                JOIN music_artists ma ON LOWER(TRIM(ma.name)) = LOWER(TRIM(p.artist_name))
                WHERE p.track_id = ?
            ";
            if let Ok(mut p_stmt) = conn.prepare(parquet_sql) {
                if let Ok(mut rows) = p_stmt.query(params![sid]) {
                    while let Ok(Some(r)) = rows.next() {
                        if let (Ok(aid), Ok(aname)) = (r.get::<_, i64>(0), r.get::<_, String>(1)) {
                            if seen_aids.insert(aid) {
                                artists.push((aid, aname));
                            }
                        }
                    }
                }
            }
        }
    }

    // Method 2: Fallback via shared album if fewer than 2 artists found
    if artists.len() < 2 {
        if let Some(alb_id) = album_id {
            let alb_sql = "
                SELECT DISTINCT ma.id, ma.name
                FROM music_albums a
                JOIN music_artists ma ON ma.id = a.artist_id
                WHERE LOWER(TRIM(a.title)) = (SELECT LOWER(TRIM(title)) FROM music_albums WHERE id = ?)
                  AND a.artist_id != ?
                LIMIT 3
            ";
            if let Ok(mut a_stmt) = conn.prepare(alb_sql) {
                if let Ok(mut rows) = a_stmt.query(params![alb_id, primary_aid]) {
                    while let Ok(Some(r)) = rows.next() {
                        if let (Ok(aid), Ok(aname)) = (r.get::<_, i64>(0), r.get::<_, String>(1)) {
                            if seen_aids.insert(aid) {
                                artists.push((aid, aname));
                            }
                        }
                    }
                }
            }
        }
    }

    // Method 3: Parse "(feat. ...)" or "(ft. ...)" from track title
    if let Some(feat_list) = extract_featured_artist_names(&title) {
        for cand in feat_list {
            if let Ok(mut f_stmt) = conn.prepare("SELECT id, name FROM music_artists WHERE LOWER(TRIM(name)) = LOWER(?) LIMIT 1") {
                if let Ok(r) = f_stmt.query_row(params![cand], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))) {
                    if seen_aids.insert(r.0) {
                        artists.push((r.0, r.1));
                    }
                }
            }
        }
    }

    Some((title, artists))
}

pub fn collect_node_tags(
    graph: &GraphCSR,
    catalog: &CatalogCache,
    node_key: &str,
    max_tags: usize,
) -> Vec<String> {
    let mut tag_edges: Vec<(i64, f32)> = graph
        .out_edges_by_key(node_key)
        .into_iter()
        .filter(|e| is_tag_relation(e.relation_type) && e.target_key.starts_with("keyword:"))
        .filter_map(|e| {
            let (_, kid) = parse_node_key(e.target_key)?;
            Some((kid, e.base_weight))
        })
        .collect();

    tag_edges.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut tags = Vec::new();
    let mut seen = HashSet::new();
    for (kid, _) in tag_edges {
        if seen.insert(kid) {
            if let Some(name) = catalog.keywords.get(&kid) {
                if crate::recommendation::context::is_steam_noise_tag(name.as_str()) {
                    continue;
                }
                tags.push(format!("#{}", name));
                if tags.len() >= max_tags {
                    break;
                }
            }
        }
    }
    tags
}
