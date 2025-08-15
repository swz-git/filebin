use std::collections::HashMap;

use askama::Template;
use axum::{
    Router,
    body::Body,
    extract::{Path, Query, Request},
    http::header::CONTENT_TYPE,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};

// TODO: database, https://github.com/kurtbuilds/ormlite?
// TODO: admin view, just send a huge table

#[derive(Template, Hash)]
#[template(path = "home.html")]
struct HomeTemplate<'a> {
    active_user: Option<&'a str>,
}

async fn home(Query(query): Query<HashMap<String, String>>) -> Html<String> {
    let tpl = HomeTemplate {
        active_user: query.get("user").map(|s| s.as_str()),
    };
    Html(tpl.render().unwrap())
}

async fn notfound(req: Request<Body>) -> impl IntoResponse {
    let path = req.uri().path();

    Response::builder()
        .header(CONTENT_TYPE, "text/html")
        .status(404)
        .body(format!("<h1>404: Not Found</h1>\n<p>{path}</p>"))
        .unwrap()
}

mod staticfiles;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let app = Router::new()
        .fallback(notfound)
        .layer(axum::middleware::from_fn(staticfiles::handler))
        .route(
            "/favicon.ico",
            get(|| async { Redirect::permanent("/filebin-ico.svg") }),
        )
        .route("/", get(home));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
