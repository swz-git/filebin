use std::{error::Error, path::PathBuf};

use async_compression::tokio::{bufread::BrotliDecoder, write::BrotliEncoder};
use bincode::{serde::decode_from_slice, Decode, Encode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_512};
use sled::Db;
use tokio::{
    fs::{self, File},
    io::{AsyncWriteExt, BufReader, BufStream},
};

use crate::AppState;

/*
# Custom database using sled

Metadata is stored with a key like this: `metadata:[ID]`
The value is just the FileInfo struct encoded with bincode
*/

#[derive(Encode, Decode, Deserialize, Serialize, PartialEq, Eq, Debug)]
pub struct FileInfo {
    /// Should be a valid mime type
    pub mime_type: String,

    /// Date of file upload
    #[bincode(with_serde)]
    pub upload_date: DateTime<Utc>,

    /// Key needed to delete the file
    pub deletion_key: String,

    /// Unique id
    pub id: String,

    /// Actual file name
    pub name: String,

    // Size of the file in bytes
    pub size: usize,
}

const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

pub fn read_file_info(id: String, db: &Db) -> Option<FileInfo> {
    let encoded_file_info: &[u8] = &db.get(format!("metadata:{}", id)).ok()??;
    let file_info: FileInfo = decode_from_slice(encoded_file_info, BINCODE_CONFIG).ok()?.0;
    log::debug!("Read file info {}", file_info.id);
    Some(file_info)
}

fn file_path_from_id(id: &str, state: &AppState) -> PathBuf {
    state.priv_config.blob_path.join(format!("{}.br", id))
}

// TODO: maybe make this buf (stream) for perf, if possible
pub async fn store_file(
    file: Vec<u8>,
    file_info: &FileInfo,
    state: &AppState,
) -> Result<(), Box<dyn Error>> {
    let encoded_file_info = bincode::encode_to_vec(&file_info, BINCODE_CONFIG)?;

    // write file to DB_PATH/blob/id.br using brotli compression
    {
        let target_file_path = file_path_from_id(&file_info.id, &state);

        let target_file = File::create(target_file_path).await?;
        let mut writer = BrotliEncoder::with_quality(
            tokio::io::BufWriter::new(target_file),
            async_compression::Level::Fastest,
        );
        writer.write_all(&file).await?;
        writer.shutdown().await?;
    }

    state
        .db
        .insert(format!("metadata:{}", file_info.id), encoded_file_info)?;
    log::debug!("Wrote file {}", file_info.id);
    Ok(())
}

pub async fn decode(
    encoded: BufReader<File>,
) -> Result<
    BufReader<async_compression::tokio::bufread::BrotliDecoder<BufStream<BufReader<File>>>>,
    Box<dyn Error>,
> {
    let reader = BufStream::new(encoded);
    let decoder = BrotliDecoder::new(reader);
    log::debug!("Decoding brotli stream");
    Ok(BufReader::new(decoder))
}

pub async fn read_file(id: String, state: &AppState) -> Option<(BufReader<File>, u64)> {
    let brotli_blob_file_path = state.priv_config.blob_path.join(format!("{}.br", id));

    let brotli_blob_file = File::open(brotli_blob_file_path).await.ok()?;

    let length = brotli_blob_file
        .metadata()
        .await
        .expect("Couldn't read file metadata")
        .len();

    let buffer = BufReader::new(brotli_blob_file);

    log::debug!("Read file {}", id);

    Some((buffer, length))
}

pub async fn delete_file(
    id: String,
    actual_deletion_key: String,
    state: &AppState,
) -> Result<bool, Box<dyn Error>> {
    let file_info = read_file_info(id, &state.db).ok_or("couldn't find file with specified id")?;

    let mut hasher = Sha3_512::new();

    hasher.update(&actual_deletion_key);
    hasher.update(&file_info.name); // salt, idk if needed but why not

    let hashed_deletion_key_raw = hasher.finalize();

    let hashed_deletion_key =
        base64::encode_config(hashed_deletion_key_raw, base64::URL_SAFE).replace('=', "");

    let valid_deletion_key = hashed_deletion_key == file_info.deletion_key;

    if !valid_deletion_key {
        return Ok(false);
    }

    let target_file_path = file_path_from_id(&file_info.id, &state);

    fs::remove_file(target_file_path).await?;

    state.db.remove(format!("metadata:{}", file_info.id))?;

    Ok(true)
}
