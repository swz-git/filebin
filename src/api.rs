use std::{
    collections::HashMap,
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    extract::{Multipart, Path, Query, State, multipart::Field},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, post},
};
use axum_extra::extract::CookieJar;
use ormlite::Model;
use tokio::io::{AsyncWriteExt, duplex};
use tracing::error;

use crate::{AppState, db::FileEntry, generate_secure_password, slug::Slug};

async fn upload(State(state): State<Arc<AppState>>, mut multipart: Multipart) -> Response {
    let maybe_file_field: Option<Field> = loop {
        let Some(field) = multipart.next_field().await.unwrap() else {
            break None;
        };
        if field.name() != Some("file") {
            continue;
        }
        break Some(field);
    };

    let Some(mut file_field) = maybe_file_field else {
        return (
            StatusCode::BAD_REQUEST,
            "Bad request: no file field in multipart",
        )
            .into_response();
    };

    let Some(file_name) = file_field.file_name().map(|x| x.to_owned()) else {
        return (
            StatusCode::BAD_REQUEST,
            "Bad request: no file name in multipart field",
        )
            .into_response();
    };
    let Some(content_type) = file_field.content_type().map(|x| x.to_owned()) else {
        return (
            StatusCode::BAD_REQUEST,
            "Bad request: no content type in multipart field",
        )
            .into_response();
    };

    let (mut sender, mut rx) = duplex(50 * 1024 * 1024);

    let t = tokio::spawn(async move {
        FileEntry {
            name: file_name,
            slug: Slug::gen_random(),
            size: 0, // This will be overwritten
            mime: content_type,
            upload_date: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
            admin_key: generate_secure_password(),
        }
        .insert_streaming(&mut rx, &mut *state.sqldb.write().await, &state.bucket)
        .await
    });

    while let Some(chunk) = file_field.chunk().await.unwrap() {
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
