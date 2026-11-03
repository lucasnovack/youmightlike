use crate::graph::GraphCSR;
use crate::ppr::{
    run_personalized_pagerank_filtered, run_personalized_pagerank_scores, top_k_from_scores,
    ScoredNode,
};
use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use duckdb::{params, params_from_iter, AccessMode, Config, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
use tower_http::{
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
};

const MAX_CACHE_SIZE: usize = 128;

#[derive(Debug, Clone)]
pub struct MovieMeta {
    pub title: String,
    pub release_year: Option<i32>,
    pub vote_average: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct GameMeta {
    pub title: String,
    pub release_year: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct BookMeta {
    pub title: String,
    pub first_publish_year: Option<i32>,
}

/// Catálogo dimensional compacto em RAM para resolução O(1) sem I/O de banco durante a montagem do subgrafo.
pub struct CatalogCache {
    pub keywords: HashMap<i64, String>,
    pub people: HashMap<i64, String>,
    pub developers: HashMap<i64, String>,
    pub authors: HashMap<i64, String>,
    pub movies: HashMap<i64, MovieMeta>,
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

        let mut games = HashMap::with_capacity(22_000);
        {
            let mut stmt = conn.prepare("SELECT id, title, release_year FROM games")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                games.insert(
                    r.get(0)?,
                    GameMeta {
                        title: r.get(1)?,
                        release_year: r.get(2).ok(),
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

// ---------------------------------------------------------------------------
// API Data Models (100% compatíveis com o contrato do frontend e FastAPI)
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

fn default_search_limit() -> usize {
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

fn default_rec_limit() -> usize {
    12
}

fn default_lang() -> String {
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

fn default_min_pop() -> i32 {
    30
}

fn default_audio_weight() -> f64 {
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
// Helpers de Topologia e Explicações em Memória
// ---------------------------------------------------------------------------

fn is_tag_relation(rel: &str) -> bool {
    matches!(rel, "has_tag" | "has_keyword" | "has_subject")
}

fn is_soundtrack_relation(rel: &str) -> bool {
    matches!(
        rel,
        "soundtrack_featured" | "soundtrack_artist" | "soundtrack_track"
    )
}

fn parse_node_key(key: &str) -> Option<(&str, i64)> {
    let (pfx, raw) = key.split_once(':')?;
    let id = raw.parse::<i64>().ok()?;
    Some((pfx, id))
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
    }
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(capitalize_first)
        .collect::<Vec<_>>()
        .join(" ")
}

fn lookup_artist_meta(conn: &Connection, aid: i64) -> Option<(String, Option<String>)> {
    conn.query_row(
        "SELECT name, country FROM music_artists WHERE id = ?",
        params![aid],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
    )
    .ok()
}

#[derive(Debug, Clone)]
struct TopTrackRow {
    pub track_id: i64,
    pub title: String,
    pub popularity: Option<i32>,
    pub tempo: Option<f64>,
    pub valence: Option<f64>,
    pub energy: Option<f64>,
}

fn lookup_top_track_full(conn: &Connection, aid: i64) -> Option<TopTrackRow> {
    conn.query_row(
        "SELECT id, title, popularity, tempo, valence, energy FROM music_tracks WHERE artist_id = ? ORDER BY popularity DESC LIMIT 1",
        params![aid],
        |r| {
            Ok(TopTrackRow {
                track_id: r.get::<_, i64>(0)?,
                title: r.get::<_, String>(1)?,
                popularity: r.get::<_, Option<i32>>(2)?,
                tempo: r.get::<_, Option<f64>>(3)?,
                valence: r.get::<_, Option<f64>>(4)?,
                energy: r.get::<_, Option<f64>>(5)?,
            })
        },
    )
    .ok()
}

fn collect_node_tags(
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
                tags.push(format!("#{}", name));
                if tags.len() >= max_tags {
                    break;
                }
            }
        }
    }
    tags
}

// ---------------------------------------------------------------------------
// Endpoints HTTP (Axum)
// ---------------------------------------------------------------------------

async fn api_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<EntityItem>>, (StatusCode, String)> {
    let q_str = params.q.trim().to_string();
    if q_str.len() < 2 {
        return Ok(Json(Vec::new()));
    }

    let entity_type = params.entity_type.as_deref().unwrap_or("");
    let limit = params.limit.max(1);

    let conn = state
        .db_conn
        .lock()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut items: Vec<EntityItem> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // 1. Artistas
    if entity_type.is_empty() || entity_type == "artist" {
        let mut variants = Vec::new();
        for v in [q_str.clone(), title_case(&q_str), capitalize_first(&q_str)] {
            if !variants.contains(&v) {
                variants.push(v);
            }
        }
        let placeholders = vec!["?"; variants.len()].join(", ");
        let sql = format!(
            "SELECT ma.id, ma.name, ma.country, ma.listeners,
                    (SELECT k.name FROM artist_tags tag_rel JOIN keywords k ON k.id = tag_rel.keyword_id WHERE tag_rel.artist_id = ma.id ORDER BY tag_rel.weight DESC LIMIT 1) as top_tag
             FROM music_artists ma
             WHERE ma.name IN ({placeholders})
                OR ma.name ILIKE ? || '%'
                OR ma.name ILIKE '%' || ? || '%'
             ORDER BY
                 CASE WHEN LOWER(ma.name) = LOWER(?) || ' (br)' THEN 1
                      WHEN LOWER(ma.name) = LOWER(?) THEN 2
                      WHEN LOWER(ma.name) LIKE LOWER(?) || ' (%)' THEN 3
                      WHEN LOWER(ma.name) LIKE LOWER(?) || '%' THEN 4
                      WHEN ma.name NOT LIKE '%;%' THEN 5
                      ELSE 6 END,
                 CASE WHEN ma.country = 'BR' THEN 1 WHEN ma.country = 'US' THEN 2 WHEN ma.country IS NOT NULL THEN 3 ELSE 4 END,
                 LENGTH(ma.name) ASC,
                 ma.listeners DESC,
                 ma.id ASC
             LIMIT 8;"
        );

        let mut bind_args: Vec<String> = variants;
        for _ in 0..6 {
            bind_args.push(q_str.clone());
        }

        if let Ok(mut stmt) = conn.prepare(&sql) {
            if let Ok(mut rows) = stmt.query(params_from_iter(bind_args.iter())) {
                while let Ok(Some(r)) = rows.next() {
                    let aid: i64 = r.get(0).unwrap_or_default();
                    let aname: String = r.get(1).unwrap_or_default();
                    let country: Option<String> = r.get(2).ok();
                    let top_tag: Option<String> = r.get(4).ok();

                    let nid = format!("artist:{}", aid);
                    if seen.insert(nid.clone()) {
                        let genre_str = top_tag.map(|t| format!(" • #{}", t)).unwrap_or_default();
                        let loc_str = country.map(|c| format!(" ({})", c)).unwrap_or_default();
                        items.push(EntityItem {
                            id: nid,
                            name: aname,
                            entity_type: "artist".to_string(),
                            subtitle: Some(format!("Artist{}{}", loc_str, genre_str)),
                            extra: None,
                        });
                    }
                }
            }
        }
    }

    // 2. Jogos
    if entity_type.is_empty() || entity_type == "game" {
        let sql = "
            SELECT id, title, release_year, positive_reviews
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

    // 3. Filmes
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

    // 4. Livros
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

    // 5. Faixas musicais
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
    Ok(Json(items))
}

pub fn compute_recommendations_sync(
    state: &AppState,
    req: &RecommendationRequest,
) -> Result<RecommendationResponse> {
    if req.seeds.is_empty() {
        return Ok(RecommendationResponse {
            seeds: Vec::new(),
            results: Vec::new(),
            graph: GraphPayload {
                nodes: Vec::new(),
                links: Vec::new(),
            },
        });
    }

    let mut sorted_seeds = req.seeds.clone();
    sorted_seeds.sort();
    let cache_key = format!(
        "{}|{}|{}|{}",
        sorted_seeds.join(","),
        req.target_type.as_deref().unwrap_or("all"),
        req.limit,
        req.lang
    );

    if let Ok(cache) = state.cache.lock() {
        if let Some(cached) = cache.get(&cache_key) {
            return Ok(cached);
        }
    }

    let seed_set: HashSet<String> = req.seeds.iter().cloned().collect();
    let mut seeds_resolved: Vec<EntityItem> = Vec::new();
    let mut diffusion_seeds: Vec<String> = Vec::new();
    let mut seed_artist_names: HashMap<String, String> = HashMap::new();
    let mut track_seed_artists: HashMap<String, (i64, String)> = HashMap::new();

    {
        let conn = state
            .db_conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;

        // 1. Resolve metadados dos nós semente
        for s in &req.seeds {
            let Some((prefix, nid)) = parse_node_key(s) else {
                continue;
            };
            match prefix {
                "artist" => {
                    if let Some((name, country)) = lookup_artist_meta(&conn, nid) {
                        seed_artist_names.insert(s.clone(), name.clone());
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name,
                            entity_type: "artist".to_string(),
                            subtitle: country,
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "movie" => {
                    if let Some(m) = state.catalog.movies.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: m.title.clone(),
                            entity_type: "movie".to_string(),
                            subtitle: m.release_year.map(|y| y.to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "game" => {
                    if let Some(g) = state.catalog.games.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: g.title.clone(),
                            entity_type: "game".to_string(),
                            subtitle: g.release_year.map(|y| y.to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "book" => {
                    if let Some(b) = state.catalog.books.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: b.title.clone(),
                            entity_type: "book".to_string(),
                            subtitle: b.first_publish_year.map(|y| y.to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "track" => {
                    if let Ok((t_title, a_name, a_id)) = conn.query_row(
                        "SELECT t.title, ma.name, t.artist_id FROM music_tracks t JOIN music_artists ma ON ma.id = t.artist_id WHERE t.id = ?",
                        params![nid],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
                    ) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: t_title,
                            entity_type: "track".to_string(),
                            subtitle: Some(a_name.clone()),
                            extra: None,
                        });
                        track_seed_artists.insert(s.clone(), (a_id, a_name.clone()));
                        let art_node = format!("artist:{}", a_id);
                        seed_artist_names.insert(art_node.clone(), a_name);
                        if !diffusion_seeds.contains(&art_node) {
                            diffusion_seeds.push(art_node);
                        }
                        if state.graph.get_node_id(s).is_some() && !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "keyword" => {
                    if let Some(kname) = state.catalog.keywords.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: format!("#{}", kname),
                            entity_type: "keyword".to_string(),
                            subtitle: Some("Tag".to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "person" => {
                    if let Some(pname) = state.catalog.people.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: pname.clone(),
                            entity_type: "person".to_string(),
                            subtitle: Some("Person".to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "developer" => {
                    if let Some(dname) = state.catalog.developers.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: dname.clone(),
                            entity_type: "developer".to_string(),
                            subtitle: Some("Studio".to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                "author" => {
                    if let Some(aname) = state.catalog.authors.get(&nid) {
                        seeds_resolved.push(EntityItem {
                            id: s.clone(),
                            name: aname.clone(),
                            entity_type: "author".to_string(),
                            subtitle: Some("Author".to_string()),
                            extra: None,
                        });
                        if !diffusion_seeds.contains(s) {
                            diffusion_seeds.push(s.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // 2. Executa a difusão PPR em memória e obtém o vetor completo de probabilidades `p`
    let active_seeds: Vec<&str> = if diffusion_seeds.is_empty() {
        req.seeds.iter().map(|s| s.as_str()).collect()
    } else {
        diffusion_seeds.iter().map(|s| s.as_str()).collect()
    };

    let p = run_personalized_pagerank_scores(&state.graph, &active_seeds, 0.85, 25, 1e-6)
        .unwrap_or_else(|_| vec![0.0; state.graph.num_nodes]);

    let mut exclude_ids: HashSet<u32> = HashSet::new();
    for s in &req.seeds {
        if let Some(uid) = state.graph.get_node_id(s) {
            exclude_ids.insert(uid);
        }
    }
    for &s in &active_seeds {
        if let Some(uid) = state.graph.get_node_id(s) {
            exclude_ids.insert(uid);
        }
    }

    // Top artistas difundidos (usados tanto para recomendar artistas/faixas quanto para pontes de trilha sonora!)
    let top_diffused_artists =
        top_k_from_scores(&state.graph, &p, &exclude_ids, "artist:", Some(20));

    let active_seed_set: HashSet<String> = active_seeds.iter().map(|s| s.to_string()).collect();
    // Conjunto de artistas de alta afinidade: sementes + top 12 artistas difundidos na cena
    let mut affinitive_artist_keys: HashSet<String> = active_seed_set
        .iter()
        .filter(|s| s.starts_with("artist:"))
        .cloned()
        .collect();
    for a in top_diffused_artists.iter().take(12) {
        affinitive_artist_keys.insert(a.node_key.clone());
    }

    // Pré-computa os mapas de vizinhos das sementes para explicações instantâneas em RAM
    let mut seed_tag_counts: HashMap<i64, usize> = HashMap::new();
    let mut seed_people: HashSet<i64> = HashSet::new();
    let mut seed_devs: HashSet<i64> = HashSet::new();
    let mut seed_authors: HashSet<i64> = HashSet::new();

    for &seed_key in &active_seeds {
        let mut seen_seed_tags = HashSet::new();
        for edge in state.graph.out_edges_by_key(seed_key) {
            if let Some((tpfx, tid)) = parse_node_key(edge.target_key) {
                if tpfx == "keyword" && is_tag_relation(edge.relation_type) {
                    if seen_seed_tags.insert(tid) {
                        *seed_tag_counts.entry(tid).or_insert(0) += 1;
                    }
                } else if tpfx == "person"
                    && matches!(edge.relation_type, "has_director" | "has_composer")
                {
                    seed_people.insert(tid);
                } else if tpfx == "developer" && edge.relation_type == "developed_by" {
                    seed_devs.insert(tid);
                } else if tpfx == "author" && edge.relation_type == "written_by" {
                    seed_authors.insert(tid);
                }
            }
        }
    }

    // Também agrega as tags mais fortes dos top 5 artistas difundidos se o perfil ainda tiver poucas tags
    if seed_tag_counts.len() < 5 {
        for a in top_diffused_artists.iter().take(5) {
            for edge in state.graph.out_edges_by_key(&a.node_key) {
                if let Some(("keyword", tid)) = parse_node_key(edge.target_key) {
                    if is_tag_relation(edge.relation_type) {
                        *seed_tag_counts.entry(tid).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let mut results: Vec<Value> = Vec::new();
    let mut graph_nodes: Vec<GraphNodePayload> = Vec::new();
    let mut graph_links: Vec<GraphLinkPayload> = Vec::new();

    // 3. Adiciona nós semente e suas âncoras estruturais ao subgrafo
    for seed_item in &seeds_resolved {
        graph_nodes.push(GraphNodePayload {
            id: seed_item.id.clone(),
            label: seed_item.name.clone(),
            node_type: "seed".to_string(),
            val: 18.0,
            tags: Vec::new(),
            desc: String::new(),
        });

        if seed_item.entity_type == "track" {
            if let Some((a_id, a_name)) = track_seed_artists.get(&seed_item.id) {
                let art_key = format!("artist:{}", a_id);
                if !graph_nodes.iter().any(|n| n.id == art_key) {
                    graph_nodes.push(GraphNodePayload {
                        id: art_key.clone(),
                        label: a_name.clone(),
                        node_type: "artist".to_string(),
                        val: 14.0,
                        tags: Vec::new(),
                        desc: String::new(),
                    });
                }
                graph_links.push(GraphLinkPayload {
                    source: seed_item.id.clone(),
                    target: art_key,
                    label: "performed_by".to_string(),
                    weight: 2.5,
                });
            }
        }

        // Âncoras estruturais para cada semente (in-memory sobre GraphCSR)
        let mut anchor_edges = state.graph.out_edges_by_key(&seed_item.id);
        anchor_edges.sort_by(|a, b| {
            let prio = |rel: &str| match rel {
                "developed_by" => 1,
                "has_director" => 2,
                "has_tag" => 3,
                _ => 4,
            };
            prio(a.relation_type)
                .cmp(&prio(b.relation_type))
                .then_with(|| {
                    b.base_weight
                        .partial_cmp(&a.base_weight)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        for edge in anchor_edges.into_iter().take(2) {
            if let Some((t_pfx, t_nid)) = parse_node_key(edge.target_key) {
                let t_label = match t_pfx {
                    "developer" => state.catalog.developers.get(&t_nid).cloned(),
                    "keyword" => state
                        .catalog
                        .keywords
                        .get(&t_nid)
                        .map(|k| format!("#{}", k)),
                    "person" => state.catalog.people.get(&t_nid).cloned(),
                    "author" => state.catalog.authors.get(&t_nid).cloned(),
                    _ => None,
                };
                if let Some(label) = t_label {
                    if !graph_nodes.iter().any(|n| n.id == edge.target_key) {
                        graph_nodes.push(GraphNodePayload {
                            id: edge.target_key.to_string(),
                            label,
                            node_type: t_pfx.to_string(),
                            val: 9.0,
                            tags: Vec::new(),
                            desc: String::new(),
                        });
                        graph_links.push(GraphLinkPayload {
                            source: seed_item.id.clone(),
                            target: edge.target_key.to_string(),
                            label: edge.relation_type.to_string(),
                            weight: edge.base_weight,
                        });
                    }
                }
            }
        }
    }

    // Adiciona pontes de tags compartilhadas entre múltiplas sementes
    if diffusion_seeds.len() > 1 {
        let mut common_tags: Vec<(i64, usize)> = seed_tag_counts
            .iter()
            .filter(|&(_, &cnt)| cnt >= 2)
            .map(|(&kid, &cnt)| (kid, cnt))
            .collect();
        common_tags.sort_by(|a, b| b.1.cmp(&a.1));

        for (kid, _) in common_tags.into_iter().take(3) {
            if let Some(kname) = state.catalog.keywords.get(&kid) {
                let b_id = format!("keyword:{}", kid);
                if !graph_nodes.iter().any(|n| n.id == b_id) {
                    graph_nodes.push(GraphNodePayload {
                        id: b_id,
                        label: format!("#{}", kname),
                        node_type: "keyword".to_string(),
                        val: 9.0,
                        tags: Vec::new(),
                        desc: String::new(),
                    });
                }
            }
        }
    }

    // Cotas balanceadas por domínio para descoberta transmidiática real (Artistas + Músicas + Filmes + Jogos)
    let (artist_quota, track_quota, movie_quota, game_quota, book_quota) =
        match req.target_type.as_deref() {
            Some("music") => {
                let a_q = (req.limit + 1) / 2;
                let t_q = req.limit.saturating_sub(a_q).max(1);
                (a_q, t_q, 0, 0, 0)
            }
            Some("movie") => (0, 0, req.limit, 0, 0),
            Some("game") => (0, 0, 0, req.limit, 0),
            Some("book") => (0, 0, 0, 0, req.limit),
            _ => {
                // Modo TODOS: garante mix transmidiático rico (Artistas, Músicas/Faixas, Filmes com trilha/estética e Jogos)
                let a_q = (req.limit * 34 / 100).max(4);
                let t_q = (req.limit * 33 / 100).max(4);
                let m_q = (req.limit * 26 / 100).max(3);
                let g_q = req.limit.saturating_sub(a_q + t_q + m_q);
                (a_q, t_q, m_q, g_q, 1)
            }
        };

    let is_pt = req.lang == "pt";
    let steam_noise_tags: HashSet<&str> = [
        "single-player",
        "multiplayer",
        "steam cloud",
        "steam achievements",
        "family sharing",
        "steam trading cards",
    ]
    .into_iter()
    .collect();

    {
        let conn = state
            .db_conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;

        // -------------------------------------------------------------------
        // A. ARTISTAS RECOMENDADOS (`artist:*`)
        // -------------------------------------------------------------------
        let mut artist_recs_added = 0usize;
        let mut recommended_artist_tracks: Vec<(i64, String, f32, TopTrackRow, String)> =
            Vec::new();

        if artist_quota > 0 || track_quota > 0 {
            for item in &top_diffused_artists {
                let nkey = &item.node_key;
                let score = item.score;
                if seed_set.contains(nkey) {
                    continue;
                }
                let Some(("artist", nid)) = parse_node_key(nkey) else {
                    continue;
                };

                if let Some((art_name, art_country)) = lookup_artist_meta(&conn, nid) {
                    let top_trk_row = lookup_top_track_full(&conn, nid);
                    let trk_info = top_trk_row.as_ref().map(|t| {
                        json!({
                            "title": t.title,
                            "popularity": t.popularity,
                            "tempo": t.tempo,
                            "valence": t.valence,
                            "energy": t.energy,
                        })
                    });

                    let mut explanation_parts: Vec<String> = Vec::new();
                    let mut direct_found: Option<(String, f32, String)> = None;

                    for &s_key in &active_seeds {
                        for e in state.graph.out_edges_by_key(s_key) {
                            if e.target_key == nkey && s_key.starts_with("artist:") && s_key != nkey
                            {
                                let s_name = seed_artist_names
                                    .get(s_key)
                                    .cloned()
                                    .or_else(|| {
                                        parse_node_key(s_key)
                                            .and_then(|(_, sid)| lookup_artist_meta(&conn, sid))
                                            .map(|(n, _)| n)
                                    })
                                    .unwrap_or_else(|| s_key.to_string());
                                direct_found =
                                    Some((e.relation_type.to_string(), e.base_weight, s_name));
                                break;
                            }
                        }
                        if direct_found.is_some() {
                            break;
                        }
                    }
                    if direct_found.is_none() {
                        for e in state.graph.out_edges_by_key(nkey) {
                            if active_seed_set.contains(e.target_key)
                                && e.target_key.starts_with("artist:")
                                && e.target_key != nkey
                            {
                                let s_name = seed_artist_names
                                    .get(e.target_key)
                                    .cloned()
                                    .or_else(|| {
                                        parse_node_key(e.target_key)
                                            .and_then(|(_, sid)| lookup_artist_meta(&conn, sid))
                                            .map(|(n, _)| n)
                                    })
                                    .unwrap_or_else(|| e.target_key.to_string());
                                direct_found =
                                    Some((e.relation_type.to_string(), e.base_weight, s_name));
                                break;
                            }
                        }
                    }

                    if let Some((rel, w, src_name)) = direct_found {
                        if rel == "similar_to" {
                            let pct = (w * 100.0).round() as i32;
                            explanation_parts.push(if is_pt {
                                format!("Co-escuta com {} ({}%)", src_name, pct)
                            } else {
                                format!("Co-listening with {} ({}%)", src_name, pct)
                            });
                        } else {
                            explanation_parts.push(if is_pt {
                                format!("Conexão direta com {}", src_name)
                            } else {
                                format!("Direct link with {}", src_name)
                            });
                        }
                    }

                    let mut shared_tags: Vec<String> = Vec::new();
                    for e in state.graph.out_edges_by_key(nkey) {
                        if is_tag_relation(e.relation_type) {
                            if let Some(("keyword", kid)) = parse_node_key(e.target_key) {
                                if seed_tag_counts.contains_key(&kid) {
                                    if let Some(kname) = state.catalog.keywords.get(&kid) {
                                        let tag_fmt = format!("#{}", kname);
                                        if !shared_tags.contains(&tag_fmt) {
                                            shared_tags.push(tag_fmt);
                                            if shared_tags.len() >= 2 {
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !shared_tags.is_empty() {
                        explanation_parts.push(format!("Tags: {}", shared_tags.join(", ")));
                    }

                    let default_reason = if is_pt {
                        "Difusão estocástica topológica"
                    } else {
                        "Topological graph diffusion"
                    };
                    let full_reason = if explanation_parts.is_empty() {
                        default_reason.to_string()
                    } else {
                        explanation_parts.join(" • ")
                    };

                    if let Some(trk) = top_trk_row {
                        recommended_artist_tracks.push((
                            nid,
                            art_name.clone(),
                            score,
                            trk,
                            full_reason.clone(),
                        ));
                    }

                    if artist_recs_added < artist_quota {
                        let country_label = art_country.as_deref().unwrap_or("Global");
                        let artist_subtitle = if is_pt {
                            format!("Artista Musical ({})", country_label)
                        } else {
                            format!("Musical Artist ({})", country_label)
                        };

                        results.push(json!({
                            "id": nkey,
                            "title": art_name,
                            "type": "artist",
                            "subtitle": artist_subtitle,
                            "score": ((score as f64) * 1000.0 * 100.0).round() / 100.0,
                            "top_track": trk_info,
                            "reason": full_reason,
                        }));
                        graph_nodes.push(GraphNodePayload {
                            id: nkey.clone(),
                            label: art_name,
                            node_type: "artist".to_string(),
                            val: 12.0,
                            tags: Vec::new(),
                            desc: String::new(),
                        });
                        artist_recs_added += 1;
                    }
                }
            }
        }

        // -------------------------------------------------------------------
        // B. MÚSICAS / FAIXAS RECOMENDADAS (`track:*`)
        // -------------------------------------------------------------------
        if track_quota > 0 {
            let mut track_recs_added = 0usize;
            let mut seen_rec_track_ids: HashSet<i64> = HashSet::new();
            let mut seen_rec_track_artists: HashSet<i64> = HashSet::new();

            // 1. Faixas diretamente difundidas no grafo (ex: trilhas sonoras de artistas que não sejam apenas repetição das sementes)
            let direct_tracks =
                top_k_from_scores(&state.graph, &p, &exclude_ids, "track:", Some(10));
            let mut seed_artist_tracks_used = 0usize;
            for item in direct_tracks {
                if track_recs_added >= track_quota {
                    break;
                }
                if seed_set.contains(&item.node_key) {
                    continue;
                }
                let Some(("track", tid)) = parse_node_key(&item.node_key) else {
                    continue;
                };
                if let Ok((t_title, a_id, a_name, t_pop, t_bpm)) = conn.query_row(
                    "SELECT t.title, ma.id, ma.name, t.popularity, t.tempo FROM music_tracks t JOIN music_artists ma ON ma.id = t.artist_id WHERE t.id = ?",
                    params![tid],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<i32>>(3)?,
                            r.get::<_, Option<f64>>(4)?,
                        ))
                    },
                ) {
                    let art_key = format!("artist:{}", a_id);
                    if active_seed_set.contains(&art_key) {
                        if seed_artist_tracks_used >= 1 {
                            continue;
                        }
                        seed_artist_tracks_used += 1;
                    }
                    if !seen_rec_track_ids.insert(tid) || !seen_rec_track_artists.insert(a_id) {
                        continue;
                    }
                    let bpm_str = t_bpm
                        .filter(|&b| b > 0.0)
                        .map(|b| format!(" • {} BPM", b.round() as i32))
                        .unwrap_or_default();
                    let trk_subtitle = if is_pt {
                        format!("Música de {}{}", a_name, bpm_str)
                    } else {
                        format!("Track by {}{}", a_name, bpm_str)
                    };
                    let trk_reason = if is_pt {
                        format!("Faixa icônica de {} conectada na trilha/cena do perfil", a_name)
                    } else {
                        format!("Iconic track by {} connected to your scene & soundtracks", a_name)
                    };

                    results.push(json!({
                        "id": item.node_key,
                        "title": t_title,
                        "type": "track",
                        "subtitle": trk_subtitle,
                        "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                        "top_track": {
                            "title": t_title,
                            "popularity": t_pop.unwrap_or(70),
                        },
                        "reason": trk_reason,
                    }));

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
                        id: item.node_key.clone(),
                        label: t_title,
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

            // 2. Faixas essenciais dos artistas difundidos na cena (garante sempre músicas recomendadas!)
            for (a_id, a_name, a_score, trk, art_reason) in recommended_artist_tracks {
                if track_recs_added >= track_quota {
                    break;
                }
                let t_key = format!("track:{}", trk.track_id);
                if seed_set.contains(&t_key)
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
                let trk_subtitle = if is_pt {
                    format!("Música de {}{}", a_name, bpm_str)
                } else {
                    format!("Track by {}{}", a_name, bpm_str)
                };
                let trk_reason = if is_pt {
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

        // -------------------------------------------------------------------
        // C. FILMES RECOMENDADOS (`movie:*`) — incluindo Trilhas Sonoras da Cena!
        // -------------------------------------------------------------------
        if movie_quota > 0 {
            let raw_movies = top_k_from_scores(&state.graph, &p, &exclude_ids, "movie:", Some(35));

            // Avalia vínculos de trilha sonora (soundtrack_featured / soundtrack_artist) com as sementes e artistas difundidos
            let mut ranked_movies: Vec<(ScoredNode, Vec<(String, String)>)> = Vec::new();
            for m_item in raw_movies {
                let mut ost_artists: Vec<(String, String)> = Vec::new();
                let mut seen_ost_names: HashSet<String> = HashSet::new();

                for e in state.graph.out_edges_by_key(&m_item.node_key) {
                    if is_soundtrack_relation(e.relation_type)
                        && e.target_key.starts_with("artist:")
                        && affinitive_artist_keys.contains(e.target_key)
                    {
                        if let Some(("artist", aid)) = parse_node_key(e.target_key) {
                            let aname = seed_artist_names
                                .get(e.target_key)
                                .cloned()
                                .or_else(|| lookup_artist_meta(&conn, aid).map(|(n, _)| n));
                            if let Some(name) = aname {
                                if seen_ost_names.insert(name.clone()) {
                                    ost_artists.push((e.target_key.to_string(), name));
                                }
                            }
                        }
                    }
                }

                // Impulsiona filmes que têm múltiplos artistas da cena na trilha sonora (ex: Singles 1992, The Crow, Into the Wild)
                let ost_boost = 1.0 + (ost_artists.len() as f32) * 0.65;
                let boosted = ScoredNode {
                    node_key: m_item.node_key,
                    score: m_item.score * ost_boost,
                };
                ranked_movies.push((boosted, ost_artists));
            }

            ranked_movies.sort_by(|a, b| {
                b.0.score
                    .partial_cmp(&a.0.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut movie_recs_added = 0usize;
            for (item, ost_artists) in ranked_movies {
                if movie_recs_added >= movie_quota {
                    break;
                }
                let nkey = &item.node_key;
                if seed_set.contains(nkey) {
                    continue;
                }
                let Some(("movie", nid)) = parse_node_key(nkey) else {
                    continue;
                };
                let Some(m) = state.catalog.movies.get(&nid) else {
                    continue;
                };

                let mut movie_explanation: Vec<String> = Vec::new();

                // 1. Trilha sonora com artistas do perfil ou da cena difundida!
                if !ost_artists.is_empty() {
                    let names: Vec<&str> = ost_artists
                        .iter()
                        .take(4)
                        .map(|(_, n)| n.as_str())
                        .collect();
                    movie_explanation.push(if is_pt {
                        format!("Trilha sonora com {}", names.join(", "))
                    } else {
                        format!("Soundtrack featuring {}", names.join(", "))
                    });

                    // Garante que pelo menos os 2 principais artistas da trilha estejam no subgrafo visual para desenhar a ponte!
                    for (art_key, art_name) in ost_artists.iter().take(2) {
                        if !graph_nodes.iter().any(|n| &n.id == art_key) {
                            graph_nodes.push(GraphNodePayload {
                                id: art_key.clone(),
                                label: art_name.clone(),
                                node_type: "artist".to_string(),
                                val: 11.5,
                                tags: Vec::new(),
                                desc: String::new(),
                            });
                        }
                    }
                }

                // 2. Diretor ou compositor compartilhado
                let mut found_person: Option<String> = None;
                let mut shared_movie_tags: Vec<String> = Vec::new();

                for e in state.graph.out_edges_by_key(nkey) {
                    if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                        if tpfx == "person" && found_person.is_none() && seed_people.contains(&tid)
                        {
                            found_person = state.catalog.people.get(&tid).cloned();
                        } else if tpfx == "keyword"
                            && is_tag_relation(e.relation_type)
                            && seed_tag_counts.contains_key(&tid)
                            && shared_movie_tags.len() < 3
                        {
                            if let Some(kname) = state.catalog.keywords.get(&tid) {
                                let tfmt = format!("#{}", kname);
                                if !shared_movie_tags.contains(&tfmt) {
                                    shared_movie_tags.push(tfmt);
                                }
                            }
                        }
                    }
                }

                if let Some(pname) = found_person {
                    movie_explanation.push(if is_pt {
                        format!("Mesmo realizador/compositor ({})", pname)
                    } else {
                        format!("Same director/composer ({})", pname)
                    });
                }
                if !shared_movie_tags.is_empty() {
                    let tag_str = shared_movie_tags.join(", ");
                    movie_explanation.push(if is_pt {
                        format!("Tags em comum: {}", tag_str)
                    } else {
                        format!("Shared tags: {}", tag_str)
                    });
                }

                let default_movie_reason = if is_pt {
                    "Afinidade de difusão de gênero e estilo"
                } else {
                    "Genre and style diffusion affinity"
                };
                let full_movie_reason = if movie_explanation.is_empty() {
                    default_movie_reason.to_string()
                } else {
                    movie_explanation.join(" • ")
                };

                let year_str = m
                    .release_year
                    .map(|y| y.to_string())
                    .unwrap_or_else(|| "N/A".to_string());
                let vote_str = m
                    .vote_average
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_else(|| "N/A".to_string());
                let movie_subtitle = if is_pt {
                    format!("Filme ({}) • Nota {}", year_str, vote_str)
                } else {
                    format!("Movie ({}) • Rating: {}", year_str, vote_str)
                };

                results.push(json!({
                    "id": nkey,
                    "title": m.title,
                    "type": "movie",
                    "subtitle": movie_subtitle,
                    "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                    "reason": full_movie_reason,
                }));
                graph_nodes.push(GraphNodePayload {
                    id: nkey.clone(),
                    label: m.title.clone(),
                    node_type: "movie".to_string(),
                    val: 12.5,
                    tags: Vec::new(),
                    desc: String::new(),
                });
                movie_recs_added += 1;
            }
        }

        // -------------------------------------------------------------------
        // D. JOGOS RECOMENDADOS (`game:*`)
        // -------------------------------------------------------------------
        if game_quota > 0 {
            let top_games =
                top_k_from_scores(&state.graph, &p, &exclude_ids, "game:", Some(game_quota + 4));
            let mut game_recs_added = 0usize;

            for item in top_games {
                if game_recs_added >= game_quota {
                    break;
                }
                let nkey = &item.node_key;
                if seed_set.contains(nkey) {
                    continue;
                }
                let Some(("game", nid)) = parse_node_key(nkey) else {
                    continue;
                };
                let Some(g) = state.catalog.games.get(&nid) else {
                    continue;
                };

                let mut game_explanation: Vec<String> = Vec::new();
                let mut found_dev: Option<String> = None;
                let mut specific_tags: Vec<String> = Vec::new();
                let mut fallback_tags: Vec<String> = Vec::new();

                for e in state.graph.out_edges_by_key(nkey) {
                    if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                        if tpfx == "developer" && found_dev.is_none() && seed_devs.contains(&tid) {
                            found_dev = state.catalog.developers.get(&tid).cloned();
                        } else if tpfx == "keyword"
                            && is_tag_relation(e.relation_type)
                            && seed_tag_counts.contains_key(&tid)
                        {
                            if let Some(kname) = state.catalog.keywords.get(&tid) {
                                let tfmt = format!("#{}", kname);
                                if !steam_noise_tags.contains(kname.as_str()) {
                                    if specific_tags.len() < 3 && !specific_tags.contains(&tfmt) {
                                        specific_tags.push(tfmt);
                                    }
                                } else if fallback_tags.len() < 2 && !fallback_tags.contains(&tfmt)
                                {
                                    fallback_tags.push(tfmt);
                                }
                            }
                        }
                    }
                }

                if let Some(dname) = found_dev {
                    game_explanation.push(if is_pt {
                        format!("Mesmo estúdio ({})", dname)
                    } else {
                        format!("Same studio ({})", dname)
                    });
                }
                if !specific_tags.is_empty() {
                    let tag_str = specific_tags.join(", ");
                    game_explanation.push(if is_pt {
                        format!("Tags em comum: {}", tag_str)
                    } else {
                        format!("Shared tags: {}", tag_str)
                    });
                } else if !fallback_tags.is_empty() {
                    game_explanation.push(format!("Tags: {}", fallback_tags.join(", ")));
                }

                let default_game_reason = if is_pt {
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
                let game_subtitle = if is_pt {
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
                game_recs_added += 1;
            }
        }

        // -------------------------------------------------------------------
        // E. LIVROS RECOMENDADOS (`book:*`)
        // -------------------------------------------------------------------
        if book_quota > 0 && results.len() < req.limit {
            let top_books =
                top_k_from_scores(&state.graph, &p, &exclude_ids, "book:", Some(book_quota + 2));
            let mut book_recs_added = 0usize;

            for item in top_books {
                if book_recs_added >= book_quota || results.len() >= req.limit {
                    break;
                }
                let nkey = &item.node_key;
                if seed_set.contains(nkey) {
                    continue;
                }
                let Some(("book", nid)) = parse_node_key(nkey) else {
                    continue;
                };
                let Some(b) = state.catalog.books.get(&nid) else {
                    continue;
                };

                let mut book_explanation: Vec<String> = Vec::new();
                let mut found_author: Option<String> = None;
                let mut shared_book_tags: Vec<String> = Vec::new();

                for e in state.graph.out_edges_by_key(nkey) {
                    if let Some((tpfx, tid)) = parse_node_key(e.target_key) {
                        if tpfx == "author" && found_author.is_none() && seed_authors.contains(&tid)
                        {
                            found_author = state.catalog.authors.get(&tid).cloned();
                        } else if tpfx == "keyword"
                            && is_tag_relation(e.relation_type)
                            && seed_tag_counts.contains_key(&tid)
                            && shared_book_tags.len() < 3
                        {
                            if let Some(kname) = state.catalog.keywords.get(&tid) {
                                let tfmt = format!("#{}", kname);
                                if !shared_book_tags.contains(&tfmt) {
                                    shared_book_tags.push(tfmt);
                                }
                            }
                        }
                    }
                }

                if let Some(aname) = found_author {
                    book_explanation.push(if is_pt {
                        format!("Mesmo autor ({})", aname)
                    } else {
                        format!("Same author ({})", aname)
                    });
                }
                if !shared_book_tags.is_empty() {
                    let tag_str = shared_book_tags.join(", ");
                    book_explanation.push(if is_pt {
                        format!("Temas em comum: {}", tag_str)
                    } else {
                        format!("Shared themes: {}", tag_str)
                    });
                }

                let default_book_reason = if is_pt {
                    "Afinidade literária e temática"
                } else {
                    "Literary and thematic affinity"
                };
                let full_book_reason = if book_explanation.is_empty() {
                    default_book_reason.to_string()
                } else {
                    book_explanation.join(" • ")
                };

                let year_str = b
                    .first_publish_year
                    .map(|y| y.to_string())
                    .unwrap_or_else(|| "N/A".to_string());
                let book_subtitle = if is_pt {
                    format!("Livro ({})", year_str)
                } else {
                    format!("Book ({})", year_str)
                };

                results.push(json!({
                    "id": nkey,
                    "title": b.title,
                    "type": "book",
                    "subtitle": book_subtitle,
                    "score": ((item.score as f64) * 1000.0 * 100.0).round() / 100.0,
                    "reason": full_book_reason,
                }));
                graph_nodes.push(GraphNodePayload {
                    id: nkey.clone(),
                    label: b.title.clone(),
                    node_type: "book".to_string(),
                    val: 12.0,
                    tags: Vec::new(),
                    desc: String::new(),
                });
                book_recs_added += 1;
            }
        }

        // -------------------------------------------------------------------
        // F. NÓS PONTE EXPLICATIVOS (`keyword:`, `person:`, `developer:`, `author:`)
        // -------------------------------------------------------------------
        let mut bridge_count = 0usize;
        let top_any = top_k_from_scores(&state.graph, &p, &exclude_ids, "", Some(120));
        for item in top_any {
            if bridge_count >= 12 {
                break;
            }
            let nkey = &item.node_key;
            let Some((prefix, nid)) = parse_node_key(nkey) else {
                continue;
            };
            if matches!(prefix, "keyword" | "person" | "developer" | "author") {
                let label = match prefix {
                    "keyword" => state
                        .catalog
                        .keywords
                        .get(&nid)
                        .map(|k| format!("#{}", k)),
                    "person" => state.catalog.people.get(&nid).cloned(),
                    "developer" => state.catalog.developers.get(&nid).cloned(),
                    "author" => state.catalog.authors.get(&nid).cloned(),
                    _ => None,
                };
                if let Some(lbl) = label {
                    if !graph_nodes.iter().any(|n| &n.id == nkey) {
                        graph_nodes.push(GraphNodePayload {
                            id: nkey.clone(),
                            label: lbl,
                            node_type: prefix.to_string(),
                            val: 8.0,
                            tags: Vec::new(),
                            desc: String::new(),
                        });
                        bridge_count += 1;
                    }
                }
            }
        }
    }

    results.truncate(req.limit);

    // 4. Extrai conexões entre os nós exibidos para o Force-Graph (100% em memória no GraphCSR!)
    let all_graph_node_ids: HashSet<String> = graph_nodes.iter().map(|n| n.id.clone()).collect();
    if !all_graph_node_ids.is_empty() {
        let mut extracted_edges: Vec<GraphLinkPayload> = Vec::new();
        for src_id in &all_graph_node_ids {
            for edge in state.graph.out_edges_by_key(src_id) {
                if all_graph_node_ids.contains(edge.target_key) {
                    extracted_edges.push(GraphLinkPayload {
                        source: src_id.clone(),
                        target: edge.target_key.to_string(),
                        label: edge.relation_type.to_string(),
                        weight: edge.base_weight,
                    });
                }
            }
        }

        let rel_prio = |rel: &str| {
            if rel.contains("soundtrack") {
                1
            } else if rel == "similar_to" {
                2
            } else if rel == "performed_by" {
                3
            } else if rel == "has_director" {
                4
            } else if rel == "developed_by" {
                5
            } else {
                6
            }
        };

        extracted_edges.sort_by(|a, b| {
            rel_prio(&a.label).cmp(&rel_prio(&b.label)).then_with(|| {
                b.weight
                    .partial_cmp(&a.weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        graph_links.extend(extracted_edges);

        // Higienização topológica: elimina nós ponte que ficaram com grau 0
        let mut connected_node_ids: HashSet<String> = HashSet::new();
        for l in &graph_links {
            connected_node_ids.insert(l.source.clone());
            connected_node_ids.insert(l.target.clone());
        }

        let rec_ids: HashSet<String> = results
            .iter()
            .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect();

        graph_nodes.retain(|n| {
            connected_node_ids.contains(&n.id)
                || n.node_type == "seed"
                || rec_ids.contains(&n.id)
        });
    }

    // 5. Enriquece cada nó do grafo com suas tags e metadados descritivos (em memória)
    {
        let conn = state
            .db_conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;

        for n in &mut graph_nodes {
            let Some((pfx, nid)) = parse_node_key(&n.id) else {
                continue;
            };

            let mut node_tags = collect_node_tags(&state.graph, &state.catalog, &n.id, 12);
            let mut node_desc = String::new();

            match pfx {
                "artist" => {
                    node_desc = if is_pt {
                        "Artista musical".to_string()
                    } else {
                        "Musical Artist".to_string()
                    };
                }
                "track" => {
                    if let Ok((a_id, a_name, bpm)) = conn.query_row(
                        "SELECT a.id, a.name, t.tempo FROM music_tracks t JOIN music_artists a ON a.id = t.artist_id WHERE t.id = ?",
                        params![nid],
                        |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, Option<f64>>(2)?,
                            ))
                        },
                    ) {
                        if node_tags.is_empty() {
                            let art_key = format!("artist:{}", a_id);
                            node_tags =
                                collect_node_tags(&state.graph, &state.catalog, &art_key, 12);
                        }
                        let bpm_str = format!("{} BPM", bpm.unwrap_or(0.0).round() as i32);
                        node_desc = if is_pt {
                            format!("Faixa de {} • {}", a_name, bpm_str)
                        } else {
                            format!("Track by {} • {}", a_name, bpm_str)
                        };
                    }
                }
                "game" => {
                    if let Some(g) = state.catalog.games.get(&nid) {
                        if let Some(y) = g.release_year {
                            node_desc = if is_pt {
                                format!("Jogo ({})", y)
                            } else {
                                format!("Game ({})", y)
                            };
                        }
                    }
                }
                "movie" => {
                    if let Some(m) = state.catalog.movies.get(&nid) {
                        let y_str = m
                            .release_year
                            .map(|y| y.to_string())
                            .unwrap_or_else(|| "N/A".to_string());
                        let r_str = m
                            .vote_average
                            .map(|v| format!("{:.1}", v))
                            .unwrap_or_else(|| "N/A".to_string());
                        node_desc = if is_pt {
                            format!("Filme ({}) • Nota {}", y_str, r_str)
                        } else {
                            format!("Movie ({}) • Rating: {}", y_str, r_str)
                        };
                    }
                }
                "book" => {
                    node_desc = if is_pt {
                        "Livro".to_string()
                    } else {
                        "Book".to_string()
                    };
                }
                "developer" => {
                    node_desc = if is_pt {
                        "Estúdio de Desenvolvimento".to_string()
                    } else {
                        "Game Developer Studio".to_string()
                    };
                }
                "keyword" => {
                    node_desc = if is_pt {
                        "Tag Conceitual".to_string()
                    } else {
                        "Conceptual Tag".to_string()
                    };
                }
                "person" => {
                    node_desc = if is_pt {
                        "Diretor / Compositor / Roteirista".to_string()
                    } else {
                        "Director / Composer / Writer".to_string()
                    };
                }
                "author" => {
                    node_desc = if is_pt {
                        "Autor Literário".to_string()
                    } else {
                        "Literary Author".to_string()
                    };
                }
                _ => {}
            }

            n.tags = node_tags;
            n.desc = node_desc;
        }
    }

    let resp_obj = RecommendationResponse {
        seeds: seeds_resolved,
        results,
        graph: GraphPayload {
            nodes: graph_nodes,
            links: graph_links,
        },
    };

    if let Ok(mut cache) = state.cache.lock() {
        cache.insert(cache_key, resp_obj.clone());
    }

    Ok(resp_obj)
}

async fn api_recommend(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RecommendationRequest>,
) -> Result<Json<RecommendationResponse>, (StatusCode, String)> {
    let resp = compute_recommendations_sync(&state, &req)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(resp))
}

/// Endpoint e função de recomendação de faixas com alinhamento acústico contínuo 100% em Rust
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

    {
        let conn = state
            .db_conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;

        for s in &req.seeds {
            let Some((pfx, id)) = parse_node_key(s) else {
                continue;
            };
            if pfx == "artist" {
                seed_artist_ids.insert(id);
            } else if pfx == "track" {
                seed_track_ids.insert(id);
                if let Ok((a_id, tempo, valence, energy, dance)) = conn.query_row(
                    "SELECT artist_id, tempo, valence, energy, danceability FROM music_tracks WHERE id = ?",
                    params![id],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, Option<f64>>(1)?,
                            r.get::<_, Option<f64>>(2)?,
                            r.get::<_, Option<f64>>(3)?,
                            r.get::<_, Option<f64>>(4)?,
                        ))
                    },
                ) {
                    seed_artist_ids.insert(a_id);
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
                'Co-escuta Last.fm com ' || ma_src.name || ' (' || CAST(ROUND(s.match_score * 100) AS INT) || '%)' AS reason
            FROM artist_similarities s
            JOIN music_artists ma_src ON ma_src.id = s.source_artist_id
            WHERE s.source_artist_id IN ({seed_ids_str})
              AND s.target_artist_id IN ({art_ids_str})
            UNION ALL
            SELECT 
                t2.artist_id,
                'Microgêneros com ' || ma_src.name || ': ' || STRING_AGG(k.name, ', ') AS reason
            FROM artist_tags t1
            JOIN music_artists ma_src ON ma_src.id = t1.artist_id
            JOIN keywords k ON k.id = t1.keyword_id
            JOIN artist_tags t2 ON t2.keyword_id = k.id AND t2.artist_id IN ({art_ids_str})
            WHERE t1.artist_id IN ({seed_ids_str})
            GROUP BY t2.artist_id, ma_src.name
        ),
        aggregated_reasons AS (
            SELECT artist_id, STRING_AGG(reason, ' • ') AS full_explanation
            FROM artist_reasons
            GROUP BY artist_id
        )
        SELECT 
            t.id AS track_id,
            t.title,
            ta.name AS artist_name,
            alb.title AS album_name,
            t.popularity,
            t.tempo,
            t.valence,
            t.energy,
            t.danceability,
            t.artist_id,
            COALESCE(ar.full_explanation, 'Conectado pela topologia estocástica do grafo cultural') AS explanation
        FROM target_artists ta
        JOIN music_tracks t ON t.artist_id = ta.id
        LEFT JOIN music_albums alb ON alb.id = t.album_id
        LEFT JOIN aggregated_reasons ar ON ar.artist_id = ta.id
        WHERE t.popularity >= {}
        ORDER BY t.popularity DESC
        LIMIT 150;",
        req.min_popularity
    );

    let mut raw_candidates = Vec::new();
    {
        let conn = state
            .db_conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
        let mut stmt = conn.prepare(&sql_candidates)?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let track_id: i64 = r.get(0)?;
            let title: String = r.get(1)?;
            let artist_name: String = r.get(2)?;
            let album_name: Option<String> = r.get(3).ok();
            let popularity: i32 = r.get(4).unwrap_or(0);
            let tempo: Option<f64> = r.get(5).ok();
            let valence: Option<f64> = r.get(6).ok();
            let energy: Option<f64> = r.get(7).ok();
            let danceability: Option<f64> = r.get(8).ok();
            let artist_id: i64 = r.get(9)?;
            let explanation: String = r.get(10)?;
            let rust_score = scored_map.get(&artist_id).copied().unwrap_or(0.001);
            raw_candidates.push((
                track_id,
                title,
                artist_name,
                album_name,
                popularity,
                tempo,
                valence,
                energy,
                danceability,
                rust_score,
                explanation,
            ));
        }
    }

    if raw_candidates.is_empty() {
        return Ok(Vec::new());
    }

    let max_graph_score = raw_candidates
        .iter()
        .map(|c| c.9)
        .fold(0.0_f64, f64::max)
        .max(1e-9);

    let mut recommendations: Vec<TrackRecommendationItem> = Vec::new();
    let mut seen_tracks: HashSet<i64> = HashSet::new();
    let mut seen_artist_titles: HashSet<(String, String)> = HashSet::new();
    let mut artist_counts: HashMap<String, usize> = HashMap::new();
    let max_per_artist = 2usize;

    for c in raw_candidates {
        let t_id = c.0;
        let t_title_norm = c.1.trim().to_lowercase();
        let art_name = c.2.clone();

        if seed_track_ids.contains(&t_id) || !seen_tracks.insert(t_id) {
            continue;
        }
        if !seen_artist_titles.insert((art_name.clone(), t_title_norm)) {
            continue;
        }
        let count = artist_counts.entry(art_name.clone()).or_insert(0);
        if *count >= max_per_artist {
            continue;
        }
        *count += 1;

        let graph_norm = (c.9 / max_graph_score).min(1.0);
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

async fn api_recommend_tracks(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TrackRecommendRequest>,
) -> Result<Json<Vec<TrackRecommendationItem>>, (StatusCode, String)> {
    let recs = compute_track_recommendations_sync(&state, &req)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(recs))
}

async fn api_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "engine": "rust-axum-csr",
        "nodes": state.graph.num_nodes,
        "edges": state.graph.num_edges,
    }))
}

pub fn load_app_state(db_path: &str) -> Result<Arc<AppState>> {
    let t0 = Instant::now();
    println!("[youmightlike-server] initializing in-memory Rust graph engine...");
    println!("  database: {}", db_path);

    let graph = GraphCSR::load_semantic_graph(db_path)?;
    println!(
        "  [1/2] loaded CSR semantic graph in {:.3}s ({} nodes, {} undirected transition edges)",
        t0.elapsed().as_secs_f32(),
        graph.num_nodes,
        graph.num_edges
    );

    let t_cat = Instant::now();
    let conn = Connection::open_with_flags(
        db_path,
        Config::default().access_mode(AccessMode::ReadOnly)?,
    )
    .context("Falha ao abrir conexão persistente DuckDB read_only")?;
    conn.execute_batch("PRAGMA threads=8; PRAGMA memory_limit='16GB';")?;

    let catalog = CatalogCache::load_from_duckdb(&conn)?;
    println!(
        "  [2/2] preloaded dimensional catalog in {:.3}s ({} keywords, {} movies, {} games, {} books)",
        t_cat.elapsed().as_secs_f32(),
        catalog.keywords.len(),
        catalog.movies.len(),
        catalog.games.len(),
        catalog.books.len()
    );

    Ok(Arc::new(AppState {
        db_path: db_path.to_string(),
        graph,
        catalog,
        db_conn: Mutex::new(conn),
        cache: Mutex::new(ResponseCache::new()),
    }))
}

pub fn build_router(state: Arc<AppState>, static_dir: PathBuf) -> Router {
    let index_file = static_dir.join("index.html");
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/health", get(api_health))
        .route("/api/search", get(api_search))
        .route("/api/recommend", post(api_recommend))
        .route("/api/recommend/tracks", post(api_recommend_tracks))
        .nest_service("/static", ServeDir::new(static_dir))
        .fallback_service(ServeFile::new(index_file))
        .layer(cors)
        .with_state(state)
}
