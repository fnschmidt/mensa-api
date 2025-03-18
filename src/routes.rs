use axum::{Router, response::IntoResponse, routing::get};
use http::{Method, header::CONTENT_TYPE};
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};

use crate::{openmensa_funcs, services, types::CanteenMealDiff};

pub async fn app(today_updated_tx: broadcast::Sender<CanteenMealDiff>) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET])
        // allow requests from any origin
        .allow_origin(Any)
        .allow_headers([CONTENT_TYPE]);

    let today_updated_id_tx = today_updated_tx.clone();
    let today_updated_diff_tx = today_updated_tx.clone();

    Router::new()
        .route("/", get(|| async { "API is reachable".into_response() }))
        .route(
            "/today_updated_ws",
            get(move |ws| services::ws_handler_today_upd_id(ws, today_updated_id_tx)),
        )
        .route(
            "/today_updated_diff_ws",
            get(move |ws| services::ws_handler_today_upd_diff(ws, today_updated_diff_tx)),
        )
        .route("/canteens", get(services::get_canteens))
        .route("/canteens/{canteen_id}", get(services::get_canteen_meta))
        .route(
            "/canteens/{canteen_id}/days",
            get(services::get_canteen_available_days),
        )
        .route(
            "/canteens/{canteen_id}/days/{date}",
            get(services::get_meals_of_day),
        )
        .route(
            "/openmensacanteens",
            get(openmensa_funcs::get_openmensa_canteens),
        )
        .layer(cors)
}
