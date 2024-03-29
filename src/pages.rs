use std::error::Error;

use axum::{
    body::boxed,
    extract::{Path, State},
    http::header,
    response::Response,
    routing::get,
    Router,
};
use chrono::Duration;
use handlebars::Handlebars;
use rust_embed::RustEmbed;
use serde_json::json;

use crate::{
    dbman,
    utils::{self, should_preview},
    AppState,
};

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
    Ok(reg.render_template(file_str, json)?)
}

async fn upload(State(state): State<AppState>) -> Response {
    let timeago = timeago::Formatter::new();

    let body = render_file(
        "upload.hbs",
        &json!({
            "maxFilesize": (&state.config.file_size_limit).get_bytes() as u64,
            "maxFilesizeReadable": state.config.file_size_limit.get_appropriate_unit(true).to_string().replace(".00", ""),
            "maxUploadPerPeriodText": format!(
                "Upload limit is {} per {}",
                state.config.ratelimit_period_byte_limit.get_appropriate_unit(true).to_string().replace(".00", ""),
                timeago.convert(Duration::seconds(state.config.ratelimit_period_length as i64).to_std().expect("couldn't calculate timeago"))
                    .replace(" ago", "")
            ),
        }),
    )
    .expect("rendering failed");

    Response::builder()
        .header(header::CONTENT_TYPE, "text/html")
        .body(boxed(body)) // I have no idea why this needs to be boxed but whatever
        .unwrap()
}

async fn file(Path(file): Path<String>, State(state): State<AppState>) -> Response {
    let uid = file;
    let maybe_info = dbman::read_file_info(uid.clone(), &state.db);
    if maybe_info.is_none() {
        return Response::builder()
            .status(404)
            .body(boxed("404".to_string())) // I have no idea why this needs to be boxed but whatever
            .unwrap();
    }
    let info = maybe_info.unwrap();

    let should_preview = should_preview(&info.mime_type, &state.config);

    let body = render_file(
        "file.hbs",
        &json!({
            "id": info.id,
            "filename": info.name,
            "img": utils::get_download_link(uid),
            "shouldPreview": should_preview,
        }),
    )
    .expect("rendering failed");

    Response::builder()
        .header(header::CONTENT_TYPE, "text/html")
        .body(boxed(body)) // I have no idea why this needs to be boxed but whatever
        .unwrap()
}

pub fn get_pages_router() -> Router<AppState> {
    Router::new()
        .route("/", get(upload))
        .route("/file/:file", get(file))
}
