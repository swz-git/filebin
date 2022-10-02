use actix_files::Files;
use actix_web::{get, web, App, HttpServer, Responder};
use log::{info, trace, warn};
use std::env;

mod api;

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

struct AppData {
    db: sled::Db,
}

#[actix_web::main] // or #[tokio::main]
async fn main() -> std::io::Result<()> {
    setup_logger().expect("Couldn't initialize logger");

    let port = 8080;
    let server = HttpServer::new(|| {
        let db = sled::open(env::var("DB_PATH").unwrap_or("./trunk_db".to_string()))
            .expect("Couldn't open database");

        App::new()
            .app_data(web::Data::new(AppData { db }))
            .route("/hello", web::get().to(|| async { "Hello World!" }))
            .configure(api::config)
    })
    .bind(("127.0.0.1", port))?
    .run();
    info!("Listening on port {}", port);
    server.await
}
