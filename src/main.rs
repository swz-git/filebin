extern crate mime_sniffer;

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
        .chain(fern::log_file("trunk.log")?)
        .apply()?;
    Ok(())
}

// TODO: add sled cache capacity to config
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AppConfig {
    file_size_limit: usize, // in kb
    allowed_preview_mime_regex: String,
    db_path: String,
    port: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            file_size_limit: 1000000, // 1000000KB == 1GB
            allowed_preview_mime_regex:
                r"^((audio|image|video)/[a-z.+-]+|(application/json|text/plain))$".to_string(),
            db_path: "./trunk_db".to_string(),
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
        .merge(Env::prefixed("TRUNK_"))
        .merge(Toml::file("trunk.toml"));

    let config: AppConfig = figment.extract().expect("Couldn't initialize config");

    let db = sled::Config::default()
        .path(&config.db_path)
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

// #[launch]
// fn rocket() -> _ {
//     setup_logger().expect("Couldn't initialize logger");

//     let figment = Figment::from(Serialized::defaults(AppConfig::default()))
//         .merge(Env::prefixed("TRUNK_"))
//         .merge(Toml::file("trunk.toml"));

//     let config: AppConfig = figment.extract().expect("Couldn't initialize config");

//     let db = sled::Config::default()
//         .path(&config.db_path)
//         .open()
//         .expect("Couldn't open database");

//     let limits = Limits::new()
//         .limit("data-form", config.file_size_limit + 100.kibibytes())
//         .limit("file", config.file_size_limit);

//     let port = 8080;
//     rocket::build()
//         .configure(Config {
//             port,
//             limits,
//             ..Config::default()
//         })
//         .manage(db)
//         .manage(config)
//         .mount("/api", api::get_routes())
//         .mount("/", pages::get_routes())
//         .mount("/", static_files::get_routes())
// }
