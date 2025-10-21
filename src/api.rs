use std::{
    collections::HashMap,
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::Body,
    extract::{Path, Query, Request, State},
    http::{StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
    routing::{delete, post},
};
use axum_extra::extract::CookieJar;
use multer::{Field, Multipart};
use ormlite::Model;
use tokio::io::{AsyncWriteExt, duplex};
use tracing::error;

use crate::{AppState, db::FileEntry, generate_secure_password, slug::Slug};

pub const DUPLEX_BUF_SIZE: usize = 100 * 1024 * 1024;

pub struct ExtractedFileInfo<'a> {
    pub name: String,
    pub mime: String,
    pub field: Field<'a>,
}

pub async fn extract_file_info<'a>(
    multipart: &mut Multipart<'a>,
) -> Result<ExtractedFileInfo<'a>, Response> {
    let maybe_file_field: Option<Field<'a>> = loop {
        let Some(field): Option<Field> = multipart.next_field().await.unwrap() else {
            break None;
        };
        // field
        if field.name() != Some("file") {
            continue;
        }
        break Some(field);
    };

    let Some(file_field) = maybe_file_field else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Bad request: no file field in multipart",
        )
            .into_response());
    };

    let Some(name) = file_field.file_name().map(|x| x.to_owned()) else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Bad request: no file name in multipart field",
        )
            .into_response());
    };
    let Some(mime) = file_field.content_type().map(|x| x.to_string()) else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Bad request: no content type in multipart field",
        )
            .into_response());
    };
    // let Some(size) = file_field..map(|x| x.to_string()) else {
    //     return Err((
    //         StatusCode::BAD_REQUEST,
    //         "Bad request: no content type in multipart field",
    //     )
    //         .into_response());
    // };

    Ok(ExtractedFileInfo {
        name,
        mime,
        field: file_field,
    })
}

pub async fn shitty_multipart_extractor<'r>(req: Request<Body>) -> Result<Multipart<'r>, Response> {
    let Ok(boundary) = multer::parse_boundary(
        req.headers()
            .get(CONTENT_TYPE)
            .map(|x| x.to_str().ok())
            .flatten()
            .unwrap_or_default(),
    ) else {
        return Err((StatusCode::BAD_REQUEST, "Bad request: invalid multipart").into_response());
    };
    let bytes = req.into_body().into_data_stream();
    Ok(Multipart::new(bytes, boundary))
}

async fn upload(State(state): State<Arc<AppState>>, req: Request<Body>) -> Response {
    let mut multipart = match shitty_multipart_extractor(req).await {
        Ok(x) => x,
        Err(e) => return e,
    };

    let ExtractedFileInfo {
        name,
        mime,
        mut field,
    } = match extract_file_info(&mut multipart).await {
        Ok(x) => x,
        Err(e) => return e,
    };

    let (mut sender, mut rx) = duplex(DUPLEX_BUF_SIZE);

    let t = tokio::spawn(async move {
        FileEntry {
            name,
            slug: Slug::gen_random(),
            size: 0, // This will be overwritten
            mime,
            upload_date: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
            admin_key: generate_secure_password(),
        }
        .insert_streaming(&mut rx, &mut *state.sqldb.write().await, &state.bucket)
        .await
    });

    while let Some(chunk) = field.chunk().await.unwrap() {
        if let Err(e) = sender.write_all(&chunk).await {
            error!("Couldn't write chunk to db: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Insert failed").into_response();
        };
    }

    // Important, tells green thread to exit
    drop(sender);

    let result = t.await.unwrap();

    if let Err(e) = result {
        error!("INSERT FAILED! {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Insert failed").into_response();
    }

    let result = result.unwrap();

    (
        StatusCode::OK,
        [(
            "Set-Cookie",
            format!(
                "FB_SLUG_KEY_{}={}; Secure; Path=/; Max-Age=315360000",
                result.slug, result.admin_key
            ),
        )],
        format!("{}", result.slug),
    )
        .into_response()
}

async fn delete_file(
    Path(slug_str): Path<String>,
    jar: CookieJar,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let Ok(slug) = Slug::from_str(&slug_str) else {
        return (StatusCode::BAD_REQUEST, "Bad request: invalid slug").into_response();
    };
    let admin_key_cookie = jar
        .iter()
        .find(|x| x.name().strip_prefix("FB_SLUG_KEY_") == Some(&slug.to_string()))
        .map(|c| c.value());
    let admin_key_param = params.get("key").map(|x| x.as_str());
    let Some(admin_key) = admin_key_cookie.or(admin_key_param) else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };

    let file = FileEntry::select()
        .where_bind("slug = ?", slug)
        .fetch_one(&mut *state.sqldb.write().await)
        .await;

    let Ok(file) = file else {
        return (
            StatusCode::BAD_REQUEST,
            "Bad request: file with slug doesn't exist",
        )
            .into_response();
    };

    if file.admin_key != admin_key {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    match file.delete(&mut *state.sqldb.write().await).await {
        Ok(_) => (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
        Err(e) => {
            error!("Failed to delete file: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR).into_response()
        }
    }
}

pub fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/file", post(upload))
        .route("/file/{slug}", delete(delete_file))
}
