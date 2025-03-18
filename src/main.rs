use constants::{CANTEEN_MAP, CANTEEN_MAP_INV};
use openmensa_funcs::init_openmensa_canteenlist;
use std::{
    env,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    signal,
    sync::{Notify, broadcast},
    time::sleep,
};

mod constants;
mod cronjobs;
mod db_operations;
mod openmensa_funcs;
mod routes;
mod services;
mod stuwe_request_funcs;
mod types;
use anyhow::Result;
use cronjobs::{start_canteen_cache_job, update_cache};
use db_operations::{check_or_create_db_tables, get_canteens_from_db};
use stuwe_request_funcs::invert_map;

#[tokio::main]
async fn main() -> Result<()> {
    if env::var(pretty_env_logger::env_logger::DEFAULT_FILTER_ENV).is_err() {
        pretty_env_logger::formatted_timed_builder()
            .filter_level(log::LevelFilter::Info)
            .init();
    } else {
        pretty_env_logger::init_timed();
    }

    log::info!("Starting Mensa API...");

    //// DB setup
    check_or_create_db_tables().unwrap();

    {
        let canteens = get_canteens_from_db().await.unwrap();
        *CANTEEN_MAP_INV.write().unwrap() = invert_map(&canteens);
        *CANTEEN_MAP.write().unwrap() = canteens;
    }

    // stuwe_request_funcs::_run_benchmark().await.unwrap();
    // return;

    tokio::spawn(async {
        if let Err(e) = init_openmensa_canteenlist().await {
            log::error!("OpenMensa list fetch failed: {}", e);
        }
    });

    // always update cache on startup
    // 'today_updated_tx' is None, sending WS updates for updated meals makes no sense
    // when the application wasnt running for an unspecified time
    match update_cache(None).await {
        Ok(_) => log::info!("Cache updated!"),
        Err(e) => log::error!("Cache update failed: {}", e),
    }

    // set up broadcast channel to notify WS clients whenever today's canteen plans changed
    let (today_updated_tx, _) = broadcast::channel(20);

    // Graceful shutdown trigger
    let shutdown_notify = Arc::new(Notify::new());
    let sched_update_task_running = Arc::new(RwLock::new(false));

    let mut scheduler =
        start_canteen_cache_job(today_updated_tx.clone(), sched_update_task_running.clone()).await;

    let listener = TcpListener::bind("0.0.0.0:9090")
        .await
        .expect("Unable to set up TcpListener");

    log::info!("Listening on {}", listener.local_addr().unwrap());

    let app = routes::app(today_updated_tx).await;

    let server =
        axum::serve(listener, app).with_graceful_shutdown(shutdown_signal(shutdown_notify));

    if let Err(e) = server.await {
        log::error!("Server error: {}", e);
    }

    // Wait for the cache job to finish
    if let Err(e) = scheduler.shutdown().await {
        log::error!("Error shutting down cache scheduler: {:?}", e);
    }

    if *sched_update_task_running.read().unwrap() {
        log::info!("Waiting for cache task completion");
    }

    while *sched_update_task_running.read().unwrap() {
        sleep(Duration::from_millis(10)).await;
    }

    log::info!("Shutdown complete.");
    Ok(())
}

/// Listens for SIGINT (Ctrl+C) and SIGTERM (docker stop)
async fn shutdown_signal(shutdown_notify: Arc<Notify>) {
    let ctrl_c = signal::ctrl_c();
    let mut terminate_handler = signal::unix::signal(signal::unix::SignalKind::terminate())
        .expect("Failed to set shutdown signal handler");
    let terminate = terminate_handler.recv();

    tokio::select! {
        _ = ctrl_c => log::info!("Received Ctrl+C, shutting down..."),
        _ = terminate => log::info!("Received SIGTERM, shutting down..."),
    }

    // Notify other tasks to shut down
    shutdown_notify.notify_waiters();
}
