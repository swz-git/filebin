use std::{env, fs};

use mime_sniffer::MimeTypeSniffer;
use regex::Regex;
use rocket::{form, fs::TempFile, http::Header, Route, State};
use sled::Db;
use uuid::Uuid;

use crate::{
    dbman::{self, FileInfo},
    utils::unique_id,
};

// TODO: rate limit, maybe based on ip? accounts (probably not)? api keys?

/// Find mime from file/contents, try and get the mime type in this order; browser, sniffer, file extension
fn find_mime_from_file(file: &TempFile, file_bin: &Vec<u8>) -> Option<String> {
    let browser_mime = file.content_type();
    if browser_mime.is_some() {
        return Some(browser_mime.unwrap().to_string());
    }

    let sniffer_mime = file_bin.sniff_mime_type();
    if sniffer_mime.is_some() {
        return Some(sniffer_mime.unwrap().to_string());
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
            name: file.name().unwrap_or("unknown").to_string(), // TODO: file extension disappears?
        },
        db,
    );

    cleanup_result.expect("Couldn't clean up temporary file");

    uid
}

#[derive(Responder)]
struct FileResponder {
    data: Vec<u8>,
    content_type: Header<'static>, // TODO: array of headers would be cleaner
    content_disposition: Header<'static>,
}

#[get("/file/<uid>")]
async fn download(uid: String, db: &State<Db>) -> Option<FileResponder> {
    let info = dbman::read_file_info(uid.clone(), db);
    let contents = dbman::read_file(uid, db);

    // TODO: Config this
    let display_filter =
        Regex::new(r"^((audio|image|video)/[a-z.+-]+|(application/json|text/plain))$").unwrap();

    let should_preview = display_filter.is_match(&info.mime_type);

    Some(FileResponder {
        data: contents,
        content_type: Header::new("Content-Type", info.mime_type),
        content_disposition: Header::new(
            "Content-Disposition",
            format!(
                "{}; filename=\"{}\"",
                if should_preview {
                    "inline"
                } else {
                    "attachment"
                },
                info.name // TODO: Extension? Filter so it won't be able to escape the ""s if that matters?
            ),
        ),
    })
}

#[get("/")]
fn index(db: &State<Db>) -> &'static str {
    log::debug!("{:?}", db.get("a").expect("shit"));
    "API Is live"
}

pub fn get_routes() -> Vec<Route> {
    routes![index, upload, download]
}
