use std::time::{SystemTime, UNIX_EPOCH};

use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use sled::Db;

use crate::dbman::{self, FileInfo};

pub fn unique_id() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let stripped_bytes: Vec<u8> = time
        .to_be_bytes()
        .into_iter()
        .filter(|&byte| byte != 0u8)
        .collect();

    base64::encode(stripped_bytes)
}

/// Given the uid and a reference to the db, get the link used to download a file.
pub fn get_download_link(uid: String, info: &FileInfo) -> String {
    const FRAGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`');

    let redirect_uri = format!(
        "/api/file/{}/{}",
        uid,
        utf8_percent_encode(&info.name, FRAGMENT)
    );

    redirect_uri
}
