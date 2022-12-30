use crate::{
    dbman::{self, FileInfo},
    utils::unique_id,
    AppConfig, AppState,
};
use axum::{
    body::{boxed, Bytes},
    extract::{multipart::MultipartError, DefaultBodyLimit, Multipart, Path, State},
    http::{
        header::{self},
        HeaderMap,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use axum_extra::body::AsyncReadBody;
use regex::Regex;
use tokio::{
    fs::File,
    io::{BufReader, BufStream},
};
use uuid::Uuid;

// TODO: rate limit, maybe based on ip? accounts (probably not)? api keys?

// since multipart consumes body, it needs to be last for some reason. introduced in axum 0.6
async fn upload(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    struct FileFieldData {
        file_name: String,
        content_type: String,
        bytes: Result<Bytes, MultipartError>,
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
            bytes: field.bytes().await,
        })
    }

    let file_field = maybe_file_field.expect("Couldn't read file from multipart");

    let uid = unique_id();

    let file_info = FileInfo {
        mime_type: file_field.content_type,
        upload_date: chrono::offset::Utc::now(),
        deletion_key: Uuid::new_v4().to_string(),
        id: uid.clone(),
        name: file_field.file_name,
    };

    dbman::store_file(
        file_field
            .bytes
            .expect("Couldn't read bytes of file")
            .try_into()
            .unwrap(),
        &file_info,
        &state,
    )
    .await
    .expect("failed to store file");

    IntoResponse::into_response(boxed(
        serde_json::to_string(&file_info).expect("failed to convert file data to json"),
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
    log::debug!("{:?}", accept_encoding.unwrap());
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
    let file_buf_reader = maybe_file.unwrap();
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
            dbman::decode(file_buf_reader).await.expect("sheise"),
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

    let display_filter = Regex::new(&state.config.allowed_preview_mime_regex).unwrap();

    let should_preview = display_filter.is_match(&info.mime_type);

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
        builder = builder.header(header::CONTENT_ENCODING, "br")
    }
    match file {
        Either::Left(x) => builder.body(x).unwrap().into_response(),
        Either::Right(x) => builder.body(x).unwrap().into_response(),
    }
}

async fn index() -> Response {
    IntoResponse::into_response("API Is live")
}

pub fn get_api_router(config: AppConfig) -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/file", post(upload))
        .route("/file/:file", get(download)) // TODO: Cache system caching files under 10mb or similar
        .layer(DefaultBodyLimit::max(config.file_size_limit * 1000 + 1000))
    // .route("/file", post(upload))
}
