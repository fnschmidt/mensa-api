use std::{env, vec};

use anyhow::Result;
use axum::Json;
use http::StatusCode;
use reqwest::Client;
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{RetryTransientMiddleware, policies::ExponentialBackoff};
use serde::Deserialize;

use crate::{
    constants::{OPENMENSA_ALL_CANTEENS, OPENMENSA_LIVE_CANTEENS},
    types::Canteen,
};

#[derive(Deserialize)]
struct Day {
    closed: bool,
}

pub async fn init_openmensa_canteenlist() -> Result<()> {
    if OPENMENSA_ALL_CANTEENS.get().is_some() {
        log::info!("OpenMensa list already initialized");
        return Ok(());
    }

    let reqwest_client = Client::builder().build().unwrap();
    let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
    let client = ClientBuilder::new(reqwest_client)
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build();

    let all_canteens: Vec<Canteen> = client
        .get("https://openmensa.org/api/v2/canteens")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    log::info!("got {} canteens", all_canteens.len());
    OPENMENSA_ALL_CANTEENS.set(all_canteens.clone()).unwrap();

    if env::var_os("FILTER_OPENMENSA").is_none() {
        return Ok(());
    };

    log::info!("Filtering OpenMensa canteen list, this might take a while");
    let mut canteens_with_days = vec![];

    for (iteration, canteen) in all_canteens.iter().enumerate() {
        if iteration % 50 == 0 {
            log::info!("{}%...", (iteration * 100) / all_canteens.len());
        }

        let days: Vec<Day> = client
            .get(format!(
                "https://openmensa.org/api/v2/canteens/{}/days",
                canteen.id
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        if !days.is_empty() && !days.iter().all(|day| day.closed) {
            canteens_with_days.push(canteen.clone());
        }
    }

    log::info!(
        "OpenMensa filtering done, {} canteens remain",
        canteens_with_days.len()
    );

    OPENMENSA_LIVE_CANTEENS.set(canteens_with_days).unwrap();
    Ok(())
}

pub async fn get_openmensa_canteens() -> Result<Json<Vec<Canteen>>, StatusCode> {
    if let Some(list) = OPENMENSA_LIVE_CANTEENS.get() {
        Ok(Json(list.clone()))
    } else if let Some(list) = OPENMENSA_ALL_CANTEENS.get() {
        log::warn!("Not filtering OpenMensa list, consider Env FILTER_OPENMENSA=y");
        Ok(Json(list.clone()))
    } else {
        log::error!("OpenMensa list not initialized");
        Ok(Json(vec![]))
    }
}
