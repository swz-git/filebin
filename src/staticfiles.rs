use axum::{body::Body, extract::Request, http::header, middleware::Next, response::IntoResponse};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "static/"]
struct Asset;

pub async fn handler(req: Request<Body>, next: Next) -> impl IntoResponse {
    let path = req.uri().path().trim_start_matches('/').to_string();

    match Asset::get(path.as_str()) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => next.run(req).await,
    }
}
