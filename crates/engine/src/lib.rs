//! youmightlike-engine: Core determinístico de difusão em grafos heterogêneos.

pub mod ppr;

/// Representação básica de um nó de recomendação
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecommendedItem {
    pub node_id: String,
    pub title: String,
    pub media_type: String,
    pub score: f64,
    pub explanation: Vec<String>,
}
