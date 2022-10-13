use std::error::Error;

use handlebars::Handlebars;
use rocket::{response::content, Route, State};
use rust_embed::RustEmbed;
use serde_json::json;
use sled::Db;

use crate::{dbman, utils};

#[derive(RustEmbed)]
#[folder = "pages"]
struct Asset;

fn render_file(filename: &str, json: &serde_json::Value) -> Result<String, Box<dyn Error>> {
    let reg = Handlebars::new();
    let file_contents: rust_embed::EmbeddedFile = match Asset::get(filename) {
        Some(x) => Ok(x),
        None => Err("Couldn't find file: ".to_string() + filename),
    }?;
    let test = file_contents.data.to_vec();
    let file_str = std::str::from_utf8(&test)?;
    Ok(reg.render_template(&file_str, json)?)
}

#[get("/")]
fn upload() -> content::RawHtml<String> {
    content::RawHtml(
        render_file("upload.hbs", &json!({"hello": "world"})).expect("rendering failed"),
    )
}

#[get("/?<file>")]
fn file(file: String, db: &State<Db>) -> content::RawHtml<String> {
    let uid = file;
    let info = dbman::read_file_info(uid.clone(), db);
    content::RawHtml(
        render_file(
            "file.hbs",
            &json!({
                "id": info.id,
                "filename": info.name,
                "img": utils::get_download_link(uid, db)
            }),
        )
        .expect("rendering failed"),
    )
}

pub fn get_routes() -> Vec<Route> {
    routes![upload, file]
}
