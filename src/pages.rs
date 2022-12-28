use std::error::Error;

use axum::{
    body::boxed,
    extract::{Path, State},
    http::header,
    response::Response,
    routing::get,
    Router,
};
use handlebars::Handlebars;
use rust_embed::RustEmbed;
use serde_json::json;

use crate::{dbman, utils, AppConfig, AppState};

#[derive(RustEmbed)]
#[folder = "pages/"]
struct Assets;

fn render_file(filename: &str, json: &serde_json::Value) -> Result<String, Box<dyn Error>> {
    let reg = Handlebars::new();
    let file_contents: rust_embed::EmbeddedFile = match Assets::get(filename) {
        Some(x) => Ok(x),
        None => Err("Couldn't find file: ".to_string() + filename),
    }?;
    let test = file_contents.data.to_vec();
    let file_str = std::str::from_utf8(&test)?;
    // TODO: inefficient?
    Ok(reg.render_template(&file_str, json)?)
}

async fn upload() -> Response {
    let body = render_file("upload.hbs", &json!({})).expect("rendering failed");

    Response::builder()
        .header(header::CONTENT_TYPE, "text/html")
        .body(boxed(body)) // I have no idea why this needs to be boxed but whatever
        .unwrap()
}

async fn file(Path(file): Path<String>, State(state): State<AppState>) -> Response {
    let uid = file;
    let maybe_info = dbman::read_file_info(uid.clone(), &state.db);
    if maybe_info == None {
        return Response::builder()
            .status(404)
            .body(boxed("404".to_string())) // I have no idea why this needs to be boxed but whatever
            .unwrap();
    }
    let info = maybe_info.unwrap();

    let body = render_file(
        "file.hbs",
        &json!({
            "id": info.id,
            "filename": info.name,
            "img": utils::get_download_link(uid)
        }),
    )
    .expect("rendering failed");

    Response::builder()
        .header(header::CONTENT_TYPE, "text/html")
        .body(boxed(body)) // I have no idea why this needs to be boxed but whatever
        .unwrap()
}

pub fn get_pages_router(config: AppConfig) -> Router<AppState> {
    Router::new()
        .route("/", get(upload))
        .route("/file/:file", get(file))
}

// pub async fn pages_handler(uri: Uri) -> Response<Full<Bytes>> {
//     let path = uri.path().trim_start_matches('/');

//     match Assets::get(path) {
//         Some(content) => {
//             let info = dbman::read_file_info(id, db)
//             let body = Full::from(Handlebars::new().render_template(content.data, &json!));
//             let mime = mime_guess::from_path(path).first_or_octet_stream();

//             Response::builder()
//                 .header(header::CONTENT_TYPE, mime.as_ref())
//                 .body(body)
//                 .unwrap()
//         }
//         None => Response::builder()
//             .status(404)
//             .body(Full::from("404"))
//             .unwrap(),
//     }
// }

// use std::error::Error;

// use handlebars::Handlebars;
// use rocket::{response::content, Route, State};
// use rust_embed::RustEmbed;
// use serde_json::json;
// use sled::Db;

// use crate::{dbman, utils};

// #[derive(RustEmbed)]
// #[folder = "pages"]
// struct Asset;

// fn render_file(filename: &str, json: &serde_json::Value) -> Result<String, Box<dyn Error>> {
//     let reg = Handlebars::new();
//     let file_contents: rust_embed::EmbeddedFile = match Asset::get(filename) {
//         Some(x) => Ok(x),
//         None => Err("Couldn't find file: ".to_string() + filename),
//     }?;
//     let test = file_contents.data.to_vec();
//     let file_str = std::str::from_utf8(&test)?;
//     Ok(reg.render_template(&file_str, json)?)
// }

// #[get("/")]
// fn upload() -> content::RawHtml<String> {
//     content::RawHtml(
//         render_file("upload.hbs", &json!({"hello": "world"})).expect("rendering failed"),
//     )
// }

// #[get("/?<file>")]
// fn file(file: String, db: &State<Db>) -> content::RawHtml<String> {
//     let uid = file;
//     let info = dbman::read_file_info(uid.clone(), db);
//     content::RawHtml(
//         render_file(
//             "file.hbs",
//             &json!({
//                 "id": info.id,
//                 "filename": info.name,
//                 "img": utils::get_download_link(uid)
//             }),
//         )
//         .expect("rendering failed"),
//     )
// }

// pub fn get_routes() -> Vec<Route> {
//     routes![upload, file]
// }
