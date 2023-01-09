use std::{
    error::Error,
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;
use sled::Batch;

use crate::{AppConfig, AppState};

// what about Db.generate_id... switch?
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

// credits are bytes.
// keys look like: ratelimit:{token}:{unix_timestamp}
// TODO: clean up ratelimit:* to remove expired keys every now and then
pub fn timebased_ratelimit(
    token: &str,
    credit_cost: u64,
    state: &AppState,
    dry: bool, // if true, don't add credit_cost to db
) -> Result<bool, Box<dyn Error>> {
    let prefix = format!("ratelimit:{}", token);
    let ratelimit_keys = state.db.scan_prefix(&prefix);
    let mut used_credits: u64 = 0;
    let mut batch = Batch::default();
    for maybe_pair in ratelimit_keys {
        let pair = maybe_pair?;
        let pay_date: DateTime<Utc> = DateTime::from_utc(
            NaiveDateTime::from_timestamp(
                String::from_utf8(
                    pair.0
                        .strip_prefix((prefix.clone() + ":").as_bytes())
                        .ok_or("couldn't strip prefix")?
                        .to_owned(),
                )?
                .parse::<i64>()?,
                0,
            ),
            Utc,
        );

        if (Utc::now() - pay_date).num_seconds() as u64 > state.config.ratelimit_period_length {
            batch.remove(pair.0)
        } else {
            used_credits += u64::from_le_bytes(
                pair.1
                    .to_vec()
                    .try_into()
                    .ok() // ugly hack to make custom error
                    .ok_or("couldn't convert bytes to u64")?,
            );
        }
    }

    state.db.apply_batch(batch)?;

    if (used_credits + credit_cost) as u128 > state.config.ratelimit_period_byte_limit.get_bytes() {
        return Ok(false);
    } else {
        if !dry {
            state.db.insert(
                format!(
                    "{}:{}",
                    prefix,
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)?
                        .as_secs()
                ),
                credit_cost.to_le_bytes().to_vec(),
            )?;
        }
        return Ok(true);
    }
}
