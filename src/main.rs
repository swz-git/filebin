#[macro_use]
extern crate rocket;
use std::env;

use rocket::Config;

mod api;
pub mod dbman;
mod static_files;

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
        .chain(fern::log_file("output.log")?)
        .apply()?;
    Ok(())
}

#[launch]
fn rocket() -> _ {
    setup_logger().expect("Couldn't initialize logger");

    let db = sled::open(env::var("DB_PATH").unwrap_or("./trunk_db".to_string()))
        .expect("Couldn't open database");

    let port = 8080;
    // info!("Listening on port {}", port);
    rocket::build()
        .configure(Config {
            port,
            ..Config::default()
        })
        .manage(db)
        .mount("/api", api::get_routes())
        .mount("/", static_files::get_routes())
}
