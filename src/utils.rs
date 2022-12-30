use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use crate::AppConfig;

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

    base64::encode_config(stripped_bytes, base64::URL_SAFE).replace('=', "")
}

/// Given the uid and a reference to the db, get the link used to download a file.
pub fn get_download_link(uid: String) -> String {
    format!("/api/file/{}", uid)
}

pub fn should_preview(mime_type: &str, config: &AppConfig) -> bool {
    let display_filter = Regex::new(&config.allowed_preview_mime_regex).unwrap();
    display_filter.is_match(mime_type)
}
