//! Axum HTTP handler endpoints and recommendation entrypoints.

use crate::models::{
    AppState, RecommendationRequest, RecommendationResponse, TrackRecommendRequest,
    TrackRecommendationItem,
};
use crate::recommendation::{
    run_recommendation_pipeline, tracks::compute_track_recommendations_sync,
};
use anyhow::Result;
use axum::{extract::State, http::StatusCode, Json};
use std::sync::Arc;

/// Computes personalized transmedia recommendations and subgraph payload synchronously
pub fn compute_recommendations_sync(
    state: &AppState,
    req: &RecommendationRequest,
) -> Result<RecommendationResponse> {
    run_recommendation_pipeline(state, req)
}

/// Axum API handler for personalized transmedia recommendations
pub async fn api_recommend(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RecommendationRequest>,
) -> Result<Json<RecommendationResponse>, (StatusCode, String)> {
    tokio::task::spawn_blocking(move || {
        compute_recommendations_sync(&state, &req)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task join error: {}", e),
        )
    })?
}

/// Axum API handler for audio-feature and graph-hybrid track recommendations
pub async fn api_recommend_tracks(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TrackRecommendRequest>,
) -> Result<Json<Vec<TrackRecommendationItem>>, (StatusCode, String)> {
    tokio::task::spawn_blocking(move || {
        compute_track_recommendations_sync(&state, &req)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task join error: {}", e),
        )
    })?
}
