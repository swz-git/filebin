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

    base64::encode(stripped_bytes)
}
