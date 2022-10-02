use bincode::{serde::decode_from_slice, Decode, Encode};
use chrono::{DateTime, Utc};
use rocket::tokio::fs::File;
use sled::Db;

/*
# Custom database using sled

Data is stored with a key like this: `trunk:storage:[ID]`
The value is just the file

Metadata is stored with a key like this: `trunk:metadata:[ID]`
The value is just the FileInfo struct encoded with bincode
*/
#[derive(Encode, Decode, PartialEq, Debug)]
pub struct FileInfo {
    /// Should be a valid mime type
    kind: String,
    #[bincode(with_serde)]
    upload_date: DateTime<Utc>,
    deletion_key: String,
    id: String,
}

const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

pub fn store_file(file: Vec<u8>, file_info: &FileInfo, db: Db) {
    let encoded_file_info =
        bincode::encode_to_vec(file_info, BINCODE_CONFIG).expect("Couldn't encode file_info");
    db.insert(format!("trunk:storage:{}", file_info.id), file)
        .expect("Failed writing file");
    db.insert(
        format!("trunk:metadata:{}", file_info.id),
        encoded_file_info,
    )
    .expect("Failed writing file info");
}

// pub fn read_file(id: String, db: Db) -> (Vec<u8>, FileInfo) {
//     let file = db
//         .get(format!("trunk:storage:{}", id))
//         .expect("Couldn't read file");
//     let encoded_file_info: Vec<u8> = db
//         .get(format!("trunk:metadata:{}", id))
//         .expect("Couldn't read file info")
//         .unwrap()
//         .into();
//     let file_info =
//         decode_from_slice(encoded_file_info, BINCODE_CONFIG).expect("Couldn't decode file info");
// }
