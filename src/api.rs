use std::collections::HashMap;

use crate::{
    dbman::{self, FileInfo},
    utils::{should_preview, timebased_ratelimit, unique_id},
    AppConfig, AppState,
};
use axum::{
    body::{boxed, Bytes},
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{
        header::{self},
        HeaderMap, StatusCode,
    },
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use axum_client_ip::ClientIp;
use axum_extra::body::AsyncReadBody;
use sha3::{Digest, Sha3_512};
use tokio::{
    fs::File,
    io::{BufReader, BufStream},
};
use uuid::Uuid;

// since multipart consumes body, it needs to be last for some reason. introduced in axum 0.6
async fn upload(
    State(state): State<AppState>,
    ClientIp(ip_address): ClientIp,
    mut multipart: Multipart,
) -> Response {
    struct FileFieldData {
        file_name: String,
        content_type: String,
        bytes: Bytes,
    }

    let mut maybe_file_field: Option<FileFieldData> = None;
    while let Some(field) = multipart.next_field().await.unwrap() {
        if field.name() != Some("file") {
            continue;
        }
        maybe_file_field = Some(FileFieldData {
            file_name: field
                .file_name()
                .expect("couldn't read file name")
                .to_string(),
            content_type: field
                .content_type()
                .expect("couldn't read content type")
                .to_string(),
            bytes: field.bytes().await.expect("Couldn't read bytes of file"),
        })
    }

    let file_field = maybe_file_field.expect("Couldn't read file from multipart");

    let uid = unique_id();

    let actual_deletion_key = Uuid::new_v4().to_string();

    let mut hasher = Sha3_512::new();

    hasher.update(&actual_deletion_key);
    hasher.update(&file_field.file_name); // salt, idk if needed but why not

    let hashed_deletion_key_raw = hasher.finalize();

    let hashed_deletion_key =
        base64::encode_config(hashed_deletion_key_raw, base64::URL_SAFE).replace('=', "");

    let file_info = FileInfo {
        mime_type: file_field.content_type,
        upload_date: chrono::offset::Utc::now(),
        deletion_key: hashed_deletion_key,
        id: uid.clone(),
        name: file_field.file_name,
        size: file_field.bytes.len(),
    };

    // TODO: should we really use ip for ratelimiting?
    let ratelimited = !timebased_ratelimit(
        &ip_address.to_string(),
        file_info.size as u64,
        &state,
        false,
    )
    .expect("couldn't check ratelimiter");

    // TODO: check ratelimiter before entire body is received
    if ratelimited {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            "Uploading this file will override your upload limit of today. Return in 24 hours.",
        )
            .into_response();
    }

    dbman::store_file(file_field.bytes.try_into().unwrap(), &file_info, &state)
        .await
        .expect("failed to store file");

    log::info!("Uploaded {} to database", &file_info.id);

    let exposed_file_info = FileInfo {
        deletion_key: actual_deletion_key, // not recoverable, hashed
        ..file_info
    };

    IntoResponse::into_response(boxed(
        serde_json::to_string(&exposed_file_info).expect("failed to convert file data to json"),
    ))
}

enum Either<L, R> {
    Left(L),
    Right(R),
}

async fn download(
    Path(uid): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response /*<tokio::io::BufReader<tokio::fs::File>>*/ {
    let accept_encoding = headers.get("Accept-Encoding");
    let use_brotli = match accept_encoding {
        None => false,
        Some(header) => match header.to_str() {
            Err(_) => false,
            Ok(x) => x.contains("br"),
        },
    };
    let maybe_file = dbman::read_file(uid.clone(), &state).await;
    if maybe_file.is_none() {
        return Response::builder()
            .status(404)
            .body(boxed("404".to_string())) // I have no idea why this needs to be boxed but whatever
            .unwrap();
    }
    let (file_buf_reader, brotli_length) = maybe_file.unwrap();
    let maybe_file: Option<
        Either<
            AsyncReadBody<BufReader<File>>,
            AsyncReadBody<
                tokio::io::BufReader<
                    async_compression::tokio::bufread::BrotliDecoder<
                        BufStream<tokio::io::BufReader<tokio::fs::File>>,
                    >,
                >,
            >,
        >,
    >;

    if !use_brotli {
        maybe_file = Some(Either::Right(AsyncReadBody::new(
            dbman::decode(file_buf_reader)
                .await
                .expect("couldn't decode brotli stream"),
        )));
    } else {
        maybe_file = Some(Either::Left(AsyncReadBody::new(file_buf_reader)));
    }

    let file = maybe_file.unwrap();

    let maybe_info = dbman::read_file_info(uid, &state.db);
    if maybe_info.is_none() {
        return Response::builder()
            .status(404)
            .body(boxed("404".to_string())) // I have no idea why this needs to be boxed but whatever
            .unwrap();
    }
    let info = maybe_info.unwrap();

    let should_preview = should_preview(&info.mime_type, &state.config);

    let mut builder = Response::builder()
        .header(header::CONTENT_TYPE, info.mime_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!(
                "{}; filename=\"{}\"",
                if should_preview {
                    "inline"
                } else {
                    "attachment"
                },
                info.name // TODO: Filter so it won't be able to escape the ""s if that matters?
            ),
        );
    if use_brotli {
        builder = builder.header(header::CONTENT_ENCODING, "br");
        builder = builder.header(header::CONTENT_LENGTH, brotli_length);
    } else {
        builder = builder.header(header::CONTENT_LENGTH, info.size)
    }
    log::info!(
        "Streaming {} to client{}",
        info.id,
        if !use_brotli {
            " using brotli decode stream"
        } else {
            ""
        }
    );
    match file {
        Either::Left(x) => builder.body(x).unwrap().into_response(),
        Either::Right(x) => builder.body(x).unwrap().into_response(),
    }
}

async fn erase(
    Path(uid): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<AppState>,
) -> Response {
    if params.get("key").is_none() {
        return Response::builder()
            .status(400)
            .body(boxed(
                "You need to provide a deletion key. DELETE /api/file/[ID]?key=[DELETION_KEY]"
                    .to_string(),
            )) // I have no idea why this needs to be boxed but whatever
            .unwrap();
    }
    let maybe_deleted = dbman::delete_file(
        uid,
        params
            .get("key")
            .ok_or("Couldn't find deletion key (should'ave been checked for above)")
            .unwrap()
            .to_owned(),
        &state,
    )
    .await;
    if maybe_deleted.is_err() {
        return Response::builder()
            .status(500)
            .body(boxed(
                "Deletion failed, maybe file doesn't exist?".to_string(),
            )) // I have no idea why this needs to be boxed but whatever
            .unwrap();
    }
    let deleted = maybe_deleted.unwrap();
    if !deleted {
        return Response::builder()
            .status(400)
            .body(boxed("Invalid deletion key".to_string())) // I have no idea why this needs to be boxed but whatever
            .unwrap();
    }
    IntoResponse::into_response("Deletion successful")
}

async fn index() -> Response {
    IntoResponse::into_response("API Is live")
}

pub fn get_api_router(config: AppConfig) -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/file", post(upload))
        .route("/file/:file", get(download)) // TODO: Cache system caching files under 10mb or similar
        .route("/file/:file", delete(erase))
        .layer(DefaultBodyLimit::max(
            (config.file_size_limit.get_bytes() + 1024) as usize,
        ))
    // .route("/file", post(upload))
}
