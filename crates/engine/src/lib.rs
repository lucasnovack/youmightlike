//! youmightlike-engine: Core determinístico de difusão em grafos heterogêneos.

pub mod graph;
pub mod ppr;

pub use graph::GraphCSR;
pub use ppr::{run_personalized_pagerank, ScoredNode};

/// Representação básica de um nó de recomendação
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecommendedItem {
    pub node_id: String,
    pub title: String,
    pub media_type: String,
    pub score: f32,
    pub explanation: Vec<String>,
}
