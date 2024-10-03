use axum::{response::IntoResponse, routing::get, Router};
use http::{header::CONTENT_TYPE, Method};
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};

use crate::{openmensa_funcs, services};

pub async fn app(today_updated_tx: broadcast::Sender<String>) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET])
        // allow requests from any origin
        .allow_origin(Any)
        .allow_headers([CONTENT_TYPE]);

    Router::new()
        .route("/", get(|| async { "API is reachable".into_response() }))
        .route(
            "/today_updated_ws",
            get(move |ws| services::ws_handler_today_upd(ws, today_updated_tx.clone())),
        )
        .route("/canteens", get(services::get_canteens))
        .route("/canteens/:canteen_id", get(services::get_canteen_meta))
        .route(
            "/canteens/:canteen_id/days",
            get(services::get_canteen_available_days),
        )
        .route(
            "/canteens/:canteen_id/days/:date",
            get(services::get_meals_of_day),
        )
        .route(
            "/openmensacanteens",
            get(openmensa_funcs::get_openmensa_canteens),
        )
        .layer(cors)
}
