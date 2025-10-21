use std::{
    convert::Infallible,
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use askama::Template;
use axum::{
    Router,
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse as _, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures::Stream;
use humansize::{DECIMAL, FormatSizeOptions, format_size};
use tokio::{
    io::{AsyncWriteExt as _, duplex},
    sync::watch,
    time::Instant,
};
use tokio_stream::{StreamExt as _, wrappers::WatchStream};
use tracing::error;

use crate::{
    AppState,
    api::{DUPLEX_BUF_SIZE, ExtractedFileInfo, extract_file_info, shitty_multipart_extractor},
    db::FileEntry,
    generate_secure_password,
    slug::Slug,
};

#[derive(Template)]
#[template(path = "components/upload.html")]
struct UploadTemplate {
    slug: String,
}
async fn init_upload(State(state): State<Arc<AppState>>) -> Response {
    let slug = Slug::gen_random();

    let (progress_sender, progress_recver) = watch::channel(
        // None means that something failed, which shouldn't be the default state.
        Some(Default::default()),
    );

    let live_upload = LiveUpload {
        slug,
        started: Instant::now(),
        progress_sender,
        progress_recver,
    };

    state.hx_state.live_uploads.write().await.push(live_upload);

    (
        StatusCode::OK,
        UploadTemplate {
            slug: slug.to_string(),
        }
        .render()
        .unwrap(),
    )
        .into_response()
}

async fn upload(
    State(state): State<Arc<AppState>>,
    Path(slug_str): Path<String>,
    req: Request<Body>,
) -> Response {
    let Ok(slug) = Slug::from_str(&slug_str) else {
        return (StatusCode::BAD_REQUEST, "Bad request: invalid slug").into_response();
    };

    let live_uploads_g = state.hx_state.live_uploads.read().await;

    let live_upload_option = live_uploads_g.iter().find(|x| x.slug == slug);

    let Some(live_upload) = live_upload_option else {
        return (
            StatusCode::NOT_FOUND,
            "Not found: Slug not initialized for upload",
        )
            .into_response();
    };

    let progress_sender = live_upload.progress_sender.clone();
    drop(live_uploads_g); // Drop guard, not sure if this is needed but why not

    let query_size_hint: Option<usize> = req.uri().query().map(|x| x.parse().ok()).flatten();

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

    let admin_key = generate_secure_password();

    let (mut sender, mut rx) = duplex(DUPLEX_BUF_SIZE);

    let admin_key_clone = admin_key.clone();
    let t = tokio::spawn(async move {
        FileEntry {
            name,
            slug,
            size: 0, // This will be overwritten
            mime,
            upload_date: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
            admin_key: admin_key_clone,
        }
        .insert_streaming(&mut rx, &mut *state.sqldb.write().await, &state.bucket)
        .await
    });

    let start_time = Instant::now();

    let mut bytes_written = 0u64;
    while let Some(chunk) = field.chunk().await.unwrap() {
        if let Err(e) = sender.write_all(&chunk).await {
            error!("Couldn't write chunk to db: {e}");
            let _ = progress_sender.send(None);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Insert failed").into_response();
        };
        bytes_written += chunk.len() as u64;
        let time_taken_so_far = start_time.elapsed().as_secs_f32();

        let size_hint = field.size_hint();
        let size_guess = (query_size_hint.unwrap_or(size_hint.1.unwrap_or(size_hint.0)) as u64)
            .max(bytes_written);
        let _ = progress_sender.send(Some(ProgressUpdate {
            bytes_per_second: bytes_written as f32 / time_taken_so_far,
            uploaded: bytes_written,
            upload_size: size_guess,
        }));
    }

    // Important, tells green thread to exit
    drop(sender);

    if let Err(e) = t.await.unwrap() {
        error!("INSERT FAILED! {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Insert failed").into_response();
    };

    (
        StatusCode::OK,
        [(
            "Set-Cookie",
            format!(
                "FB_SLUG_KEY_{}={}; Secure; Path=/; Max-Age=315360000",
                slug, admin_key
            ),
        )],
        "Done!",
    )
        .into_response()
}

#[derive(Template)]
#[template(path = "components/upload_bar.html")]
struct UploadBarTemplate {
    progress: ProgressUpdate,
    human_speed: String,
    human_percentage: String,
}
async fn progress(
    State(state): State<Arc<AppState>>,
    Path(slug_str): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Response> {
    let Ok(slug) = Slug::from_str(&slug_str) else {
        return Err((StatusCode::BAD_REQUEST, "Bad request: invalid slug").into_response());
    };

    let live_uploads_g = state.hx_state.live_uploads.read().await;

    let live_upload_option = live_uploads_g.iter().find(|x| x.slug == slug);

    let Some(live_upload) = live_upload_option else {
        return Err((
            StatusCode::NOT_FOUND,
            "Not found: Slug not initialized for upload",
        )
            .into_response());
    };

    let progress_recver = live_upload.progress_recver.clone();
    drop(live_uploads_g); // Drop guard, not sure if this is needed but why not
    let maybe_last = progress_recver.borrow();
    if let Some(last) = &*maybe_last
        && last.upload_size == last.uploaded
    {
        return Err((StatusCode::OK, "Done.").into_response());
    }
    drop(maybe_last);

    let stream = WatchStream::new(progress_recver)
        .map(|x| match x {
            Some(prog) => {
                if prog.upload_size != prog.uploaded {
                    UploadBarTemplate {
                        human_speed: format_size(
                            prog.bytes_per_second as u32 * 8,
                            FormatSizeOptions::from(DECIMAL).base_unit(humansize::BaseUnit::Bit),
                        ),
                        human_percentage: format!(
                            "~{:2.1}%",
                            prog.uploaded as f32 / prog.upload_size as f32 * 100.
                        ),
                        progress: prog,
                    }
                    .render()
                    .unwrap()
                } else {
                    "<h2>Finalizing...</h2><h4>Please be patient</h4>".into()
                }
            }
            None => "<h1>Upload failed.</h1><a href=\"/\">Try again</a>".into(),
        })
        .map(|x| Ok(Event::default().event("msg").data(x)));

    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

#[derive(Debug, Clone)]
struct ProgressUpdate {
    bytes_per_second: f32,
    uploaded: u64,
    upload_size: u64,
}

impl Default for ProgressUpdate {
    fn default() -> Self {
        Self {
            bytes_per_second: 0.,
            uploaded: 0,
            upload_size: 1,
        }
    }
}

#[derive(Debug)]
struct LiveUpload {
    slug: Slug,
    started: Instant,
    progress_sender: tokio::sync::watch::Sender<Option<ProgressUpdate>>,
    progress_recver: tokio::sync::watch::Receiver<Option<ProgressUpdate>>,
}

#[derive(Debug, Default)]
pub struct HxState {
    live_uploads: tokio::sync::RwLock<Vec<LiveUpload>>,
}

pub fn hx_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/file/{slug}", post(upload))
        .route("/progress/{slug}", get(progress))
        .route("/init", get(init_upload))
}
