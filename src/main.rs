use std::{
    fs,
    path::{Path, PathBuf},
};

use api::get_api_router;
use axum::Router;
use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use pages::get_pages_router;
use serde::{Deserialize, Serialize};
use sled::Db;
use static_files::static_handler;

mod api;
pub mod dbman;
mod pages;
mod static_files;
pub mod utils;

#[cfg(debug_assertions)]
fn get_log_level() -> log::LevelFilter {
    log::LevelFilter::Debug
}

#[cfg(not(debug_assertions))]
fn get_log_level() -> log::LevelFilter {
    log::LevelFilter::Info
}

fn setup_logger() -> Result<(), fern::InitError> {
    let log_level = get_log_level();

    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(log_level)
        .chain(std::io::stdout())
        .chain(fern::log_file("filebin.log")?)
        .apply()?;
    Ok(())
}

// TODO: add sled cache capacity to config
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AppConfig {
    /// in kb
    file_size_limit: usize,
    allowed_preview_mime_regex: String,
    db_path: PathBuf,
    /// in kb
    sled_cache_cap: usize,
    port: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            file_size_limit: 1_000_000, // 1000000KB == 1GB
            allowed_preview_mime_regex:
                r"^((audio|image|video)/[a-z.+-]+|(application/json|text/plain))$".to_string(),
            db_path: Path::new("./filebin_db").to_path_buf(),
            sled_cache_cap: 500_000, // 500_000KB = .5GB
            port: 8080,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrivAppConfig {
    sled_path: PathBuf,
    blob_path: PathBuf,
}

#[derive(Clone)]
pub struct AppState {
    db: Db,
    config: AppConfig,
    priv_config: PrivAppConfig,
}

#[tokio::main]
async fn main() {
    setup_logger().expect("Couldn't initialize logger");

    log::info!("Loading config...");

    let figment = Figment::from(Serialized::defaults(AppConfig::default()))
        .merge(Env::prefixed("FILEBIN_"))
        .merge(Toml::file("filebin.toml"));

    let config: AppConfig = figment.extract().expect("Couldn't initialize config");
    let priv_config = PrivAppConfig {
        sled_path: config.db_path.join("sled"),
        blob_path: config.db_path.join("blob"),
    };

    log::info!("Opening database...");

    fs::create_dir_all(&priv_config.blob_path).expect("Couldn't create blob database directory");
    fs::create_dir_all(&priv_config.sled_path).expect("Couldn't create sled database directory");

    let db = sled::Config::default()
        .path(&priv_config.sled_path)
        .cache_capacity((config.sled_cache_cap * 1000) as u64)
        .open()
        .expect("Couldn't open database");

    let app_state = AppState {
        db,
        config: config.clone(),
        priv_config,
    };

    log::info!("Building router...");

    // build our application with a single route
    let app = Router::new()
        .nest("/", get_pages_router())
        .nest("/api", get_api_router(config.clone()))
        // .route("/", get(|| async { "Hello, World!" }))
        .fallback(static_handler)
        .with_state(app_state);

    let address = format!("0.0.0.0:{}", config.port);
    let server = axum::Server::bind(&address.parse().unwrap()).serve(app.into_make_service());
    log::info!("Serving filebin on {}", address);
    server.await.unwrap();
}
