//! Main

use std::{
    sync::{Arc, LazyLock, Mutex},
    thread,
};

use clap::Parser;

mod app;
use app::*;

mod app_config;
use app_config::*;

mod threadpool;
use threadpool::*;
use winit::error::EventLoopError;

static CONFIG: LazyLock<AppConfig> = LazyLock::new(|| AppConfig::parse());

/// Entry point for the application.
fn main() -> Result<(), EventLoopError> {
    println!("Running with {} threads", CONFIG.threads());

    // Initialize logger for tao.
    env_logger::init();

    // Create a thread pool for rendering tiles in parallel.
    let pool = Arc::new(Mutex::new(ThreadPool::build(CONFIG.threads()).unwrap()));

    // Track remaining tiles. It will be used to shutdown the thread pool.
    let remaining_tiles = Arc::new(Mutex::new(CONFIG.tiles()));

    // Start a separate thread that will queue all tiles. This way we can run the event loop in main thread.
    {
        let pool = Arc::clone(&pool);
        thread::spawn(|| render(pool, remaining_tiles));
    }

    // Run the event loop.
    run_event_loop()
}
