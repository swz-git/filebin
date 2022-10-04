use std::{env, fs};

use rocket::{form, fs::TempFile, http::ContentType, Route, State};
use sled::Db;
use uuid::Uuid;

use crate::{
    dbman::{self, FileInfo},
    utils::unique_id,
};

// TODO: rate limit, maybe based on ip? accounts (probably not)? api keys?

#[post("/file", data = "<file>")]
async fn upload(mut file: form::Form<TempFile<'_>>, db: &State<Db>) -> String {
    let uid = unique_id();
    let file_path = env::temp_dir().join(uid.clone() + ".UNSAFE.file");
    file.persist_to(&file_path)
        .await
        .expect("Failed to create temporary file");
    let file_bin = fs::read(&file_path).expect("Couldn't read file");
    dbman::store_file(
        file_bin,
        &FileInfo {
            mime_type: file
                .content_type()
                .expect("Couldn't parse mime type for file")
                .to_string(),
            upload_date: chrono::offset::Utc::now(),
            deletion_key: Uuid::new_v4().to_string(),
            id: uid.clone(),
            name: file
                .name()
                .expect("Couldn't parse name for file")
                .to_string(),
        },
        db,
    );
    fs::remove_file(file_path).expect("Couldn't clean up temporary file");
    uid
}

// TODO: Optimize this, for some reason it's pretty slow. At least when running ddosify
#[get("/file/download/<uid>")]
async fn download(uid: String, db: &State<Db>) -> Option<(ContentType, Vec<u8>)> {
    let info = dbman::read_file_info(uid.clone(), db);
    let mime = info.mime_type;
    let split_mime: Vec<&str> = mime.split('/').collect();
    let content_type = ContentType::new(split_mime[0].to_string(), split_mime[1].to_string());

    Some((content_type, dbman::read_file(uid, db)))
}

#[get("/")]
fn index(db: &State<Db>) -> &'static str {
    log::debug!("{:?}", db.get("a").expect("shit"));
    "API Is live"
}

pub fn get_routes() -> Vec<Route> {
    routes![index, upload, download]
}
