use axum::{
    body::Body,
    extract::Request,
    http::{StatusCode, header},
    middleware::Next,
    response::IntoResponse,
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "static/"]
struct Asset;

pub async fn handler(req: Request<Body>, next: Next) -> impl IntoResponse {
    let path = req.uri().path().trim_start_matches('/').to_string();

    let Some(content) = Asset::get(path.as_str()) else {
        return next.run(req).await;
    };

    let hash = format!(
        "ahash_{:x}",
        ahash::RandomState::with_seed(420).hash_one(&content.data)
    );

    if req
        .headers()
        .get(header::IF_NONE_MATCH)
        .map(|x| x.to_str().unwrap())
        == Some(&hash)
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    let mime = mime_guess::from_path(path).first_or_octet_stream();
    (
        [(header::CONTENT_TYPE, mime.as_ref()), (header::ETAG, &hash)],
        content.data,
    )
        .into_response()
}
