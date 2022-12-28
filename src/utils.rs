use std::time::{SystemTime, UNIX_EPOCH};

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

    base64::encode_config(stripped_bytes, base64::URL_SAFE).replace("=", "")
}

/// Given the uid and a reference to the db, get the link used to download a file.
pub fn get_download_link(uid: String) -> String {
    format!("/api/file/{}", uid)
}
