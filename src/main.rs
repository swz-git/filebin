use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use api::get_api_router;
use axum::{response::Redirect, routing::get, Router};
use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use owo_colors::OwoColorize;
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
                "{} {} [{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.target().dimmed(),
                match record.level() {
                    log::Level::Debug => log::Level::Debug.to_string().dimmed().to_string(),
                    log::Level::Error => log::Level::Error.to_string().red().on_black().to_string(),
                    log::Level::Info => log::Level::Info.to_string().bright_blue().to_string(),
                    log::Level::Trace => log::Level::Trace.to_string(),
                    log::Level::Warn => log::Level::Warn.to_string().yellow().to_string(),
                },
                message
            ))
        })
        .level(log_level)
        .chain(std::io::stdout())
        .chain(fern::log_file("filebin.log")?)
        .apply()?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AppConfig {
    file_size_limit: byte_unit::Byte,
    /// Length of period in seconds
    ratelimit_period_length: u64,
    /// Byte limit you can upload every ratelimit_period_length seconds.
    ratelimit_period_byte_limit: byte_unit::Byte,
    allowed_preview_mime_regex: String,
    db_path: PathBuf,
    sled_cache_cap: byte_unit::Byte,
    port: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            file_size_limit: byte_unit::Byte::from_str("1 GiB").unwrap(),
            ratelimit_period_length: 60 * 60 * 24, // One day
            ratelimit_period_byte_limit: byte_unit::Byte::from_str("2 GiB").unwrap(),
            allowed_preview_mime_regex:
                r"^((audio|image|video)/[a-z.+-]+|(application/json|text/plain))$".to_string(),
            db_path: Path::new("./filebin_db").to_path_buf(),
            sled_cache_cap: byte_unit::Byte::from_str("0.5 GiB").unwrap(),
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

// TODO: graceful shutdown?
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
        .cache_capacity(config.sled_cache_cap.get_bytes() as u64)
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
        .route(
            "/favicon.ico",
            get(|| async { Redirect::permanent("/favicon.svg") }),
        )
        .fallback(static_handler)
        .with_state(app_state);

    let address = format!("0.0.0.0:{}", config.port);
    let server = axum::Server::bind(&address.parse().unwrap())
        .serve(app.into_make_service_with_connect_info::<SocketAddr>());
    log::info!("Serving filebin on {}", address);
    server.await.unwrap();
}
