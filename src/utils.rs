use std::time::{SystemTime, UNIX_EPOCH};

// TODO: this returns weird ids? example: AAAAAAAAAAAXGvgt+3qcRw==
pub fn unique_id() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_be_bytes();

    base64::encode(time)
}
