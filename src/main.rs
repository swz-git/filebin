use std::{env, fs, path::PathBuf, str::FromStr as _, sync::Arc};

use askama::Template;
use axum::{
    Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CONTENT_TYPE, SERVER, USER_AGENT, WWW_AUTHENTICATE},
    },
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use axum_extra::extract::CookieJar;
use base64::{Engine, prelude::BASE64_URL_SAFE};
use chrono::DateTime;
use color_eyre::eyre::{self, Context};
use humansize::BINARY;
use object_store::{ObjectStore as _, aws::AmazonS3, path::Path as ObjPath};
use ormlite::Model;
use rand::Rng as _;
use sqlx::{Connection as _, SqliteConnection};
use tokio::sync::RwLock;
use tower_http::compression::CompressionLayer;
use tracing::{debug, error, info};

use crate::{
    api::api_router,
    db::FileEntry,
    hx::{HxState, hx_router},
    slug::Slug,
};
mod api;
mod db;
mod hx;
mod slug;

#[derive(Debug, Clone)]
struct SafeFileEntry {
    pub name: String,
    pub slug: String, // see Slug struct
    pub size: String, // sqlite doesn't understand u32s
    pub upload_date: String,
    pub mime: String,
}

impl From<FileEntry> for SafeFileEntry {
    fn from(value: FileEntry) -> Self {
        Self {
            name: value.name,
            slug: value.slug.to_string(),
            size: humansize::format_size(value.size as u32, BINARY),
            upload_date: DateTime::from_timestamp_millis(value.upload_date)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string(),
            mime: value.mime,
        }
    }
}

#[derive(Template)]
#[template(path = "pages/home.html")]
struct HomeTemplate {
    files: Vec<SafeFileEntry>,
}

async fn home(State(state): State<Arc<AppState>>, jar: CookieJar) -> Response {
    let slugs = jar
        .iter()
        .filter_map(|pair| pair.name().strip_prefix("FB_SLUG_KEY_"))
        .filter_map(|slug_str| Slug::from_str(slug_str).ok())
        .collect::<Vec<_>>();

    let files = match db::get_slugs(&mut *state.sqldb.write().await, slugs.iter()).await {
        Ok(x) => x,
        Err(e) => {
            error!("Couldn't fetch all files: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR).into_response();
        }
    };

    let mut set_cookie_headers: HeaderMap = Default::default();

    for received_slug in slugs.iter() {
        if files.iter().find(|x| &x.slug == received_slug).is_none() {
            set_cookie_headers.append(
                "Set-Cookie",
                format!("FB_SLUG_KEY_{received_slug}=INVALID; Max-Age=0")
                    .parse()
                    .unwrap(),
            );
        }
    }

    let tpl = HomeTemplate {
        files: files.into_iter().map(Into::into).collect(),
    };
    let mut res = Html(tpl.render().unwrap()).into_response();
    res.headers_mut().extend(set_cookie_headers);
    res
}

// TODO: make ts good
#[derive(Template)]
#[template(path = "pages/admin.html")]
struct AdminTemplate {
    files: Vec<db::FileEntry>,
}

pub fn verify_admin_auth(auth_header_value: &HeaderValue, admin_pswd_hash: &str) -> bool {
    argon2::verify_encoded(
        admin_pswd_hash,
        auth_header_value
            .to_str()
            .ok()
            .and_then(|x| x.strip_prefix("Basic ").to_owned())
            .and_then(|x| BASE64_URL_SAFE.decode(x).ok())
            .map(|x| String::from_utf8_lossy(&x).into())
            .and_then(|x: String| x.split_once(":").map(|x| x.1.to_owned()))
            .unwrap_or("".to_owned())
            .as_bytes(),
    )
    .unwrap_or(false)
}

async fn admin(State(state): State<Arc<AppState>>, req: Request) -> Response {
    if let Some(header) = req.headers().get("Authorization")
        && verify_admin_auth(header, &state.admin_pswd_hash)
    {
        let files = match FileEntry::select()
            .fetch_all(&mut *state.sqldb.write().await)
            .await
        {
            Ok(x) => x,
            Err(e) => {
                error!("Couldn't fetch all files: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR).into_response();
            }
        };

        let tpl = AdminTemplate { files };
        (StatusCode::OK, Html(tpl.render().unwrap())).into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(WWW_AUTHENTICATE, "Basic charset=\"UTF-8\"")],
        )
            .into_response()
    }
}

#[derive(Template)]
#[template(path = "pages/file_page.html")]
struct FilePageTemplate {
    file: SafeFileEntry,
    dl_url: String,
}
async fn file_page(Path(slug_str): Path<String>, State(state): State<Arc<AppState>>) -> Response {
    let Ok(slug) = Slug::from_str(&slug_str) else {
        return (StatusCode::BAD_REQUEST, "Bad request: invalid slug").into_response();
    };
    let file_result = FileEntry::select()
        .where_("slug = ?")
        .bind(slug)
        .fetch_one(&mut *state.sqldb.write().await)
        .await;

    let Ok(file) = file_result else {
        return (StatusCode::NOT_FOUND, "Bad request: slug not found").into_response();
    };

    let Ok(dl_url) = file.get_s3_dl_link(&state.bucket).await else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Couldn't get object download link",
        )
            .into_response();
    };

    let tpl = FilePageTemplate {
        file: file.into(),
        dl_url,
    };
    (StatusCode::OK, Html(tpl.render().unwrap())).into_response()
}

async fn notfound() -> impl IntoResponse {
    Response::builder()
        .header(CONTENT_TYPE, "text/html")
        .status(404)
        .body(format!("<h1>404: Not Found</h1>"))
        .unwrap()
}

mod staticfiles;

#[derive(Debug)]
struct AppState {
    hx_state: HxState,
    sqldb: RwLock<SqliteConnection>,
    // db_age: X, // TODO: Do not sweep s3 files older than our sqlite db.
    bucket: AmazonS3,
    admin_pswd_hash: String,
}

const DEFAULT_FILEBIN_DB_URL: &str = "./filebin.db";

fn generate_secure_password() -> String {
    let mut rng = rand::rng();
    (&mut rng)
        .sample_iter(rand::distr::Alphanumeric)
        .take(40)
        .map(char::from)
        .collect()
}

fn get_admin_pswd_hash() -> String {
    env::var("FILEBIN_ARGON").unwrap_or_else(|_| {
        let pswd = generate_secure_password();
        info!(
            "NO ADMIN PASSWORD PROVIDED! \
                Use the `FILEBIN_ARGON` environment variable to provide a password. \
                Generate a password using `echo -n \"mypassword\" | argon2 \"filebin_\" -id -e`"
        );
        info!("GENERATED PASSWORD FOR THIS SESSION: {pswd}");
        argon2::hash_encoded(
            pswd.as_bytes(),
            b"filebin_",
            &argon2::Config {
                variant: argon2::Variant::Argon2id,
                ..Default::default()
            },
        )
        .unwrap()
    })
}

// TODO: Ratelimiting 1GiB per day, unless admin?
// TODO: Move old...
// TODO: /hx/ api where we serve html for htmx. would be nice for progress bars etc.

#[tokio::main(flavor = "multi_thread")]
async fn main() -> eyre::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt::init();

    let admin_pswd_hash = get_admin_pswd_hash();

    let db_url = env::var("FILEBIN_DB_URL").unwrap_or_else(|_| {
        let full_path = PathBuf::from(DEFAULT_FILEBIN_DB_URL);
        fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&full_path)
            .unwrap();
        info!(
            "Using default db url `{DEFAULT_FILEBIN_DB_URL}` (`{}`)",
            full_path.canonicalize().unwrap().display()
        );
        full_path.to_string_lossy().into()
    });

    let mut sqldb = sqlx::SqliteConnection::connect(&db_url).await.unwrap();

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS file_entries (
            name TEXT NOT NULL,
            slug TEXT PRIMARY KEY NOT NULL,
            size INTEGER NOT NULL,
            upload_date INTEGER NOT NULL,
            mime TEXT NOT NULL,
            admin_key TEXT NOT NULL
        );"#,
    )
    .execute(&mut sqldb)
    .await
    .context("Failed to create file_entries table")?;

    // Connect to bucket
    let s3_endpoint = env::var("S3_ENDPOINT").context("Couldn't find S3_ENDPOINT.")?;
    let s3_access_key = env::var("S3_ACCESS_KEY").context("Couldn't find S3_ACCESS_KEY.")?;
    let s3_secret_key = env::var("S3_SECRET_KEY").context("Couldn't find S3_SECRET_KEY.")?;
    let s3_bucket = env::var("S3_BUCKET").context("Couldn't find S3_BUCKET.")?;

    let bucket = object_store::aws::AmazonS3Builder::new()
        .with_endpoint(&s3_endpoint)
        .with_bucket_name(&s3_bucket)
        .with_access_key_id(s3_access_key)
        .with_secret_access_key(s3_secret_key)
        .build()?;

    if let Err(e) = bucket.head(&ObjPath::from("/")).await {
        return Err(e).context("Couldn't list bucket contents.");
    };

    info!(
        "Connected to S3-compatible server. {}/{}",
        s3_endpoint, s3_bucket
    );

    let state = Arc::new(AppState {
        hx_state: Default::default(),
        sqldb: RwLock::new(sqldb),
        admin_pswd_hash,
        bucket,
    });

    let (_sweeper_thread, _sweeper_killer) = db::spawn_sweeper(state.clone()).await;

    // router passes through from bottom to top
    let app = Router::new()
        .fallback(notfound)
        .layer(axum::middleware::from_fn(staticfiles::handler))
        .route(
            "/favicon.ico",
            get(|| async { Redirect::permanent("/filebin-ico.svg") }),
        )
        .route("/", get(home))
        .route("/f/{slug}", get(file_page))
        .route("/admin", get(admin))
        // EVERYTHING ABOVE GETS COMPRESSED! ^^^
        .layer(CompressionLayer::new().quality(tower_http::CompressionLevel::Fastest))
        .nest("/api", api_router())
        .nest("/hx", hx_router())
        // Add cool header 😎
        .layer(axum::middleware::from_fn(
            async |req: Request, next: axum::middleware::Next| {
                debug!(
                    "{} {} agent: {:?}",
                    req.method().as_str(),
                    req.uri(),
                    req.headers()
                        .get(USER_AGENT)
                        .map(|v| v.to_str().unwrap())
                        .unwrap_or("None")
                );
                let mut res = next.run(req).await;
                res.headers_mut()
                    .insert(SERVER, HeaderValue::from_static("swz/filebin"));
                res
            },
        ))
        // 10GiB Hard limit
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024 * 1024))
        .with_state(state);

    let addr = env::var("FILEBIN_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    info!("Listening on {addr}");
    axum::serve(listener, app).await.unwrap();
    Ok(())
}
