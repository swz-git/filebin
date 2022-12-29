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
    db_path: String,
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
            db_path: "./filebin_db".to_string(),
            sled_cache_cap: 500_000, // 500_000KB = .5GB
            port: 8080,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    db: Db,
    config: AppConfig,
}

#[tokio::main]
async fn main() {
    setup_logger().expect("Couldn't initialize logger");

    let figment = Figment::from(Serialized::defaults(AppConfig::default()))
        .merge(Env::prefixed("FILEBIN_"))
        .merge(Toml::file("filebin.toml"));

    let config: AppConfig = figment.extract().expect("Couldn't initialize config");

    let db = sled::Config::default()
        .path(&config.db_path)
        .cache_capacity((config.sled_cache_cap * 1000) as u64)
        .open()
        .expect("Couldn't open database");

    let app_state = AppState {
        db,
        config: config.clone(),
    };

    // build our application with a single route
    let app = Router::new()
        .nest("/", get_pages_router())
        .nest("/api", get_api_router(config.clone()))
        // .route("/", get(|| async { "Hello, World!" }))
        .fallback(static_handler)
        .with_state(app_state);

    let address = format!("0.0.0.0:{}", config.port);
    axum::Server::bind(&address.parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
