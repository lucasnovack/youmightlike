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
    conn.query_row(
        "SELECT t.id, t.title, a.title, t.popularity, t.tempo, t.valence, t.energy
         FROM music_tracks t
         LEFT JOIN music_albums a ON a.id = t.album_id
         WHERE t.artist_id = ?
         ORDER BY t.popularity DESC NULLS LAST
         LIMIT 1",
        params![aid],
        |r| {
            Ok(TopTrackRow {
                track_id: r.get::<_, i64>(0)?,
                title: r.get::<_, String>(1)?,
                album_title: r.get::<_, Option<String>>(2)?,
                popularity: r.get::<_, Option<i32>>(3)?,
                tempo: r.get::<_, Option<f64>>(4)?,
                valence: r.get::<_, Option<f64>>(5)?,
                energy: r.get::<_, Option<f64>>(6)?,
            })
        },
    )
    .ok()
}

pub fn lookup_seed_top_tracks(conn: &Connection, aid: i64, limit: usize) -> Vec<TopTrackRow> {
    let mut stmt = match conn.prepare(
        "SELECT t.id, t.title, a.title, t.popularity, t.tempo, t.valence, t.energy
         FROM music_tracks t
         LEFT JOIN music_albums a ON a.id = t.album_id
         WHERE t.artist_id = ?
         ORDER BY t.popularity DESC NULLS LAST
         LIMIT ?",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = stmt
        .query_map(params![aid, limit as i64], |r| {
            Ok(TopTrackRow {
                track_id: r.get::<_, i64>(0)?,
                title: r.get::<_, String>(1)?,
                album_title: r.get::<_, Option<String>>(2)?,
                popularity: r.get::<_, Option<i32>>(3)?,
                tempo: r.get::<_, Option<f64>>(4)?,
                valence: r.get::<_, Option<f64>>(5)?,
                energy: r.get::<_, Option<f64>>(6)?,
            })
        })
        .ok();

    if let Some(r_iter) = rows {
        r_iter.filter_map(|r| r.ok()).collect()
    } else {
        Vec::new()
    }
}

pub fn lookup_artist_collaborative_track(
    conn: &Connection,
    candidate_aid: i64,
    seed_album_titles: &std::collections::HashMap<String, (i64, String)>,
) -> Option<(TopTrackRow, i64, String)> {
    if seed_album_titles.is_empty() {
        return None;
    }

    let mut stmt = conn
        .prepare(
            "SELECT t.id, t.title, a.title, t.popularity, t.tempo, t.valence, t.energy
             FROM music_tracks t
             JOIN music_albums a ON a.id = t.album_id
             WHERE t.artist_id = ?
             ORDER BY t.popularity DESC NULLS LAST",
        )
        .ok()?;

    let rows = stmt
        .query_map(params![candidate_aid], |r| {
            Ok(TopTrackRow {
                track_id: r.get::<_, i64>(0)?,
                title: r.get::<_, String>(1)?,
                album_title: r.get::<_, Option<String>>(2)?,
                popularity: r.get::<_, Option<i32>>(3)?,
                tempo: r.get::<_, Option<f64>>(4)?,
                valence: r.get::<_, Option<f64>>(5)?,
                energy: r.get::<_, Option<f64>>(6)?,
            })
        })
        .ok()?;

    for row in rows.flatten() {
        if let Some(ref alb) = row.album_title {
            let norm = alb.trim().to_lowercase();
            if norm.len() >= 3 && norm != "single" && norm != "ep" && norm != "greatest hits" {
                if let Some((seed_aid, seed_name)) = seed_album_titles.get(&norm) {
                    if *seed_aid != candidate_aid {
                        return Some((row, *seed_aid, seed_name.clone()));
                    }
                }
            }
        }
    }

    None
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
