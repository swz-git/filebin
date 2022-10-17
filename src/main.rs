#[macro_use]
extern crate rocket;
use std::env;

extern crate mime_sniffer;

use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use rocket::{
    data::{ByteUnit, Limits, ToByteUnit},
    Config,
};
use serde::{Deserialize, Serialize};

// TODO: Memory leak? upload file + ddosify get file = memory go brrrrrr
// Seems to be sled which is insanely slow at getting data when the database is big
// fuck sled? https://github.com/search?l=Rust&o=desc&q=key+value&s=stars&type=Repositories
// looks cool: https://github.com/nomic-io/merk

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

#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct AppConfig {
    file_size_limit: ByteUnit,
    allowed_preview_mime_regex: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            file_size_limit: 1.gibibytes(),
            allowed_preview_mime_regex:
                r"^((audio|image|video)/[a-z.+-]+|(application/json|text/plain))$".to_string(),
        }
    }
}

#[launch]
fn rocket() -> _ {
    setup_logger().expect("Couldn't initialize logger");

    let figment = Figment::from(Serialized::defaults(AppConfig::default()))
        .merge(Env::prefixed("TRUNK_"))
        .merge(Toml::file("trunk.toml"));

    let config: AppConfig = figment.extract().expect("Couldn't initialize config");

    log::debug!("AAAA: {}", config.file_size_limit);

    let db = sled::open(env::var("DB_PATH").unwrap_or("./trunk_db".to_string()))
        .expect("Couldn't open database");

    let limits = Limits::new()
        .limit("data-form", config.file_size_limit + 100.kibibytes())
        .limit("file", config.file_size_limit);

    let port = 8080;
    rocket::build()
        .configure(Config {
            port,
            limits,
            ..Config::default()
        })
        .manage(db)
        .manage(config)
        .mount("/api", api::get_routes())
        .mount("/", pages::get_routes())
        .mount("/", static_files::get_routes())
}
