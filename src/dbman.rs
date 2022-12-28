use std::error::Error;

use bincode::{serde::decode_from_slice, Decode, Encode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sled::Db;

/*
# Custom database using sled

Data is stored with a key like this: `trunk:storage:[ID]`
The value is just the file

Metadata is stored with a key like this: `trunk:metadata:[ID]`
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
}

const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

pub fn store_file(file: Vec<u8>, file_info: &FileInfo, db: &Db) -> Result<(), Box<dyn Error>> {
    let encoded_file_info = bincode::encode_to_vec(file_info, BINCODE_CONFIG)?;
    db.insert(format!("trunk:storage:{}", file_info.id), file)?;
    db.insert(
        format!("trunk:metadata:{}", file_info.id),
        encoded_file_info,
    )?;
    log::debug!("Write file {}", file_info.id);
    Ok(())
}

pub fn read_file_info(id: String, db: &Db) -> Option<FileInfo> {
    let encoded_file_info: &[u8] = &db.get(format!("trunk:metadata:{}", id)).ok()??;
    let file_info: FileInfo = decode_from_slice(encoded_file_info, BINCODE_CONFIG).ok()?.0;
    log::debug!("Read file info {}", file_info.id);
    Some(file_info)
}

pub fn read_file(id: String, db: &Db) -> Option<Vec<u8>> {
    let x = db.get(format!("trunk:storage:{}", id)).ok()??.to_vec();
    log::debug!("Read file {}", id);
    Some(x)
}

// TODO: delete file function
