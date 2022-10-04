use rocket::{http::ContentType, Route};
use rust_embed::RustEmbed;
use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[derive(RustEmbed)]
#[folder = "static"]
struct Asset;

#[get("/<file..>")]
fn dist(file: PathBuf) -> Option<(ContentType, Cow<'static, [u8]>)> {
    let mut filename = file.display().to_string();
    log::debug!("{}", filename);
    let mut maybe_asset = Asset::get(&filename);
    if maybe_asset.is_none() {
        if filename.is_empty() {
            filename += "index.html";
        } else {
            filename += "/index.html";
        }
        maybe_asset = Asset::get(&filename);
    }
    let asset = maybe_asset?;
    let content_type = Path::new(&filename)
        .extension()
        .and_then(OsStr::to_str)
        .and_then(ContentType::from_extension)
        .unwrap_or(ContentType::Bytes);

    Some((content_type, asset.data))
}

pub fn get_routes() -> Vec<Route> {
    routes![dist]
}
