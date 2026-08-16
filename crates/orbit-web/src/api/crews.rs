//! Crew registry handlers.

use crate::state::Ws;
use axum::response::{IntoResponse, Json, Response};

pub(super) async fn list_crews(Ws(runtime): Ws) -> Response {
    Json(runtime.configured_crew_registry_projection()).into_response()
}
