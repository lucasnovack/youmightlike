//! youmightlike-engine: Deterministic heterogeneous graph diffusion core and HTTP service.

#![allow(clippy::type_complexity, clippy::too_many_arguments)]

pub mod graph;
pub mod models;
pub mod ppr;
pub mod recommendation;
pub mod recommender;
pub mod search;
pub mod service;

pub use graph::{DirectedEdge, GraphCSR};
pub use ppr::{
    run_personalized_pagerank, run_personalized_pagerank_filtered,
    run_personalized_pagerank_multi_hit, run_personalized_pagerank_scores,
    top_k_from_scores, ScoredNode,
};

/// Basic representation of a recommended node
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecommendedItem {
    pub node_id: String,
    pub title: String,
    pub media_type: String,
    pub score: f32,
    pub explanation: Vec<String>,
}
