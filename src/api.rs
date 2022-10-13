use std::{env, fs};

use either::*;
use mime_sniffer::MimeTypeSniffer;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use rocket::{form, fs::TempFile, http::ContentType, response::Redirect, Route, State};
use sled::Db;
use uuid::Uuid;

use crate::{
    dbman::{self, FileInfo},
    utils::{get_download_link, unique_id},
};

// TODO: rate limit, maybe based on ip? accounts (probably not)? api keys?

// TODO: fix weird arg types?
/// Find mime from file/contents, try and get the mime type in this order; browser, sniffer, file extension
fn find_mime_from_file(file: &form::Form<TempFile>, file_bin: &Vec<u8>) -> Option<String> {
    let browser_mime = file.content_type();
    if browser_mime.is_some() {
        return Some(browser_mime.unwrap().to_string());
    }

    let sniffer_mime = file_bin.sniff_mime_type();
    if sniffer_mime.is_some() {
        return Some(sniffer_mime.unwrap().to_string());
    }

    let file_name = file.name();
    if file_name.is_some() {
        let ext_mime = mime_guess::from_path(file_name.unwrap()).first();
        if ext_mime.is_some() {
            return Some(ext_mime.unwrap().to_string());
        }
    }

    None
}

// TODO: This fails on bigger files, bigger than about 1mb
#[post("/file", data = "<file>")]
async fn upload(mut file: form::Form<TempFile<'_>>, db: &State<Db>) -> String {
    let uid = unique_id();
    let file_path = env::temp_dir().join(uid.clone() + ".UNSAFE.file");
    file.persist_to(&file_path)
        .await
        .expect("Failed to create temporary file");
    let file_bin = fs::read(&file_path).expect("Couldn't read file");
    let cleanup_result = fs::remove_file(file_path); // try to clean up before potential error

    let mime_type = find_mime_from_file(&file, &file_bin).unwrap_or("text/plain".to_string());

    dbman::store_file(
        file_bin,
        &FileInfo {
            mime_type,
            upload_date: chrono::offset::Utc::now(),
            deletion_key: Uuid::new_v4().to_string(),
            id: uid.clone(),
            name: file.name().unwrap_or(&"unknown").to_string(),
        },
        db,
    );

    cleanup_result.expect("Couldn't clean up temporary file");

    uid
}

// TODO: Optimize this, for some reason it's pretty slow. At least when running ddosify https://github.com/flamegraph-rs/flamegraph
// TODO: Cookies can be accessed with js running in uploaded html file, this is a security risk
#[get("/file/<uid>/<filename>")]
async fn download(
    uid: String,
    filename: Option<String>,
    db: &State<Db>,
) -> Either<Redirect, Option<(ContentType, Vec<u8>)>> {
    let info = dbman::read_file_info(uid.clone(), db);

    if info.name != filename.unwrap_or(info.name.clone()) {
        // TODO: get_download_link gets FileInfo from the db, but we already have FileInfo in the info variable here. This wastes a database call.
        let redirect_uri = get_download_link(uid, db);

        return Left(Redirect::to(redirect_uri.to_string()));
    }

    let mime = info.mime_type;
    let split_mime: Vec<&str> = mime.split('/').collect();
    let content_type = ContentType::new(split_mime[0].to_string(), split_mime[1].to_string());

    Right(Some((content_type, dbman::read_file(uid, db))))
}

#[get("/file/<uid>")]
async fn redirect_download(uid: String, db: &State<Db>) -> Redirect {
    let redirect_uri = get_download_link(uid, db);

    Redirect::to(redirect_uri.to_string())
}

#[get("/")]
fn index(db: &State<Db>) -> &'static str {
    log::debug!("{:?}", db.get("a").expect("shit"));
    "API Is live"
}

pub fn get_routes() -> Vec<Route> {
    routes![index, upload, download, redirect_download]
}
