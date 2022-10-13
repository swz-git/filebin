#[macro_use]
extern crate rocket;
use std::env;

extern crate mime_sniffer;

use rocket::{
    data::{ByteUnit, Limits},
    Config,
};

// TODO: Memory leak? upload file + ddosify get file = memory go brrrrrr

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

#[launch]
fn rocket() -> _ {
    setup_logger().expect("Couldn't initialize logger");

    let db = sled::open(env::var("DB_PATH").unwrap_or("./trunk_db".to_string()))
        .expect("Couldn't open database");

    let one_gib: ByteUnit = "1GiB".parse().unwrap();

    // TODO: config this
    let limits = Limits::new().limit("file", one_gib); // 1gb

    let port = 8080;
    rocket::build()
        .configure(Config {
            port,
            limits,
            ..Config::default()
        })
        .manage(db)
        .mount("/api", api::get_routes())
        .mount("/", pages::get_routes())
        .mount("/", static_files::get_routes())
}
