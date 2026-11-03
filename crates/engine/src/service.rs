//! Servidor HTTP Axum e orquestração de endpoints para o youmightlike-engine.
//!
//! Este módulo atua como o ponto de entrada das rotas HTTP, delegando a busca
//! e resolução de seeds para `search`, o cálculo de difusão transmidiática para
//! `recommender`, e os tipos e estado compartilhado para `models`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use axum::{
    extract::State,
    response::Json,
    routing::{get, post},
    Router,
};
use duckdb::{AccessMode, Config, Connection};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

pub use crate::models::{
    AppState, BookMeta, CatalogCache, EntityItem, GameMeta, MovieMeta, RecommendationRequest,
    RecommendationResponse, ResponseCache, SearchParams, SeriesMeta, TrackRecommendRequest,
    TrackRecommendationItem,
};
pub use crate::recommender::{api_recommend, api_recommend_tracks};
pub use crate::search::api_search;

/// Resposta do endpoint de health
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub nodes: usize,
    pub edges: usize,
}

/// Handler do endpoint de verificação de integridade e métricas básicas de nós/arestas.
pub async fn api_health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        nodes: state.graph.num_nodes,
        edges: state.graph.num_edges,
    })
}

/// Carrega o grafo CSR, o catálogo e monta o estado de aplicação compartilhado (`AppState`).
pub fn load_app_state(db_path: &str) -> Result<Arc<AppState>> {
    println!("[server] loading graph from duckdb at {}...", db_path);
    let t0 = Instant::now();
    let graph = crate::graph::GraphCSR::load_semantic_graph(db_path)?;
    println!(
        "[server] graph loaded in {:?}: {} nodes, {} edges",
        t0.elapsed(),
        graph.num_nodes,
        graph.num_edges
    );

    println!("[server] opening duckdb connection for metadata and search...");
    let conn = Connection::open_with_flags(
        db_path,
        Config::default().access_mode(AccessMode::ReadOnly)?,
    )
    .context("Falha ao abrir conexão DuckDB para AppState")?;

    println!("[server] caching metadata catalogs...");
    let t_meta = Instant::now();
    let catalog = CatalogCache::load_from_duckdb(&conn)?;
    println!(
        "[server] catalog loaded in {:?}: {} movies, {} series, {} games, {} books",
        t_meta.elapsed(),
        catalog.movies.len(),
        catalog.series.len(),
        catalog.games.len(),
        catalog.books.len()
    );

    let state = Arc::new(AppState {
        db_path: db_path.to_string(),
        graph,
        catalog,
        db_conn: Mutex::new(conn),
        cache: Mutex::new(ResponseCache::new()),
    });

    Ok(state)
}

/// Constrói o roteador Axum com suporte a CORS, arquivos estáticos e fallback da SPA.
pub fn build_router(state: Arc<AppState>, static_dir: PathBuf) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = Router::new()
        .route("/health", get(api_health))
        .route("/search", get(api_search))
        .route("/recommend", post(api_recommend))
        .route("/recommend/tracks", post(api_recommend_tracks));

    let index_file = static_dir.join("index.html");

    Router::new()
        .nest("/api", api_routes)
        .fallback_service(
            ServeDir::new(&static_dir).fallback(axum::routing::get_service(
                tower_http::services::ServeFile::new(index_file),
            )),
        )
        .layer(cors)
        .with_state(state)
}
