//! youmightlike-engine: Core determinístico de difusão em grafos heterogêneos e serviço HTTP.

pub mod graph;
pub mod models;
pub mod ppr;
pub mod recommender;
pub mod search;
pub mod service;

pub use graph::{DirectedEdge, GraphCSR};
pub use ppr::{
    run_personalized_pagerank, run_personalized_pagerank_filtered,
    run_personalized_pagerank_scores, top_k_from_scores, ScoredNode,
};

/// Representação básica de um nó de recomendação
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecommendedItem {
    pub node_id: String,
    pub title: String,
    pub media_type: String,
    pub score: f32,
    pub explanation: Vec<String>,
}
