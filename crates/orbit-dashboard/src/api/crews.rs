//! Crew registry handlers.

use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Json, Response};
use orbit_core::OrbitRuntime;

pub(super) async fn list_crews(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    Json(runtime.configured_crew_registry_projection()).into_response()
}
