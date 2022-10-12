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

pub fn store_file(file: Vec<u8>, file_info: &FileInfo, db: &Db) {
    let encoded_file_info =
        bincode::encode_to_vec(file_info, BINCODE_CONFIG).expect("Couldn't encode file_info");
    db.insert(format!("trunk:storage:{}", file_info.id), file)
        .expect("Failed writing file");
    db.insert(
        format!("trunk:metadata:{}", file_info.id),
        encoded_file_info,
    )
    .expect("Failed writing file info");
    log::info!("Write file {}", file_info.id);
}

pub fn read_file_info(id: String, db: &Db) -> FileInfo {
    let encoded_file_info: &[u8] = &db
        .get(format!("trunk:metadata:{}", id))
        .expect("Couldn't read file info")
        .unwrap();
    let file_info: FileInfo = decode_from_slice(encoded_file_info, BINCODE_CONFIG)
        .expect("Couldn't decode file info")
        .0;
    log::info!("Read file {}", file_info.id);
    file_info
}

pub fn read_file(id: String, db: &Db) -> Vec<u8> {
    db.get(format!("trunk:storage:{}", id))
        .expect("Couldn't read file")
        .unwrap()
        .to_vec()
}

// TODO: delete file function
