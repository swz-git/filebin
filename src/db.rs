use std::{mem, str::FromStr, sync::Arc, time::Duration};

use axum::{body::Bytes, http::Method};
use color_eyre::eyre::{self, Context};
use futures::StreamExt as _;
use object_store::{
    Attribute, AttributeValue, Attributes, ObjectStore as _, PutMultipartOptions, PutPayload,
    aws::AmazonS3, path::Path as ObjPath,
};
use ormlite::Model;
use sqlx::SqliteConnection;
use tokio::{
    io::{AsyncRead, AsyncReadExt as _},
    select,
    sync::oneshot,
    task::JoinHandle,
    time::Instant,
};
use tracing::{debug, error, info};

use crate::{AppState, slug::Slug};

#[derive(Model, Debug)]
#[ormlite(table = "file_entries")]
pub struct FileEntry {
    pub name: String,
    #[ormlite(primary_key)]
    pub slug: Slug, // see Slug struct
    pub size: i64, // sqlite doesn't understand u32s
    pub upload_date: i64,
    pub mime: String,
    pub admin_key: String, // uuidv4
}

pub async fn get_slugs(
    db: &mut SqliteConnection,
    slugs: impl IntoIterator<Item = &Slug>,
) -> Result<Vec<FileEntry>, ormlite::SqlxError> {
    let collected_slugs: Vec<&Slug> = slugs.into_iter().collect();
    let placeholders = std::iter::repeat("?")
        .take(collected_slugs.len())
        .collect::<Vec<_>>()
        .join(",");

    let query_str = format!(
        "SELECT * FROM file_entries WHERE slug IN ({}) ORDER BY upload_date DESC",
        placeholders
    );

    let mut query = FileEntry::query(&query_str);

    for slug in collected_slugs {
        query = query.bind(slug)
    }

    query.fetch_all(db).await
}

impl FileEntry {
    fn get_obj_path(&self) -> ObjPath {
        ObjPath::from(format!("/{}", self.slug))
    }

    pub async fn insert_streaming<R>(
        mut self,
        reader: &mut R,
        db: &mut SqliteConnection,
        bucket: &AmazonS3,
    ) -> eyre::Result<FileEntry>
    where
        R: AsyncRead + Unpin,
    {
        let mut multipart = bucket
            .put_multipart_opts(
                &self.get_obj_path(),
                PutMultipartOptions {
                    attributes: Attributes::from_iter(
                        [(
                            Attribute::ContentType,
                            AttributeValue::from(self.mime.clone()),
                        )]
                        .into_iter(),
                    ),
                    ..Default::default()
                },
            )
            .await?;

        const CHUNK_SIZE: usize = 25 * 1024 * 1024;

        let mut buf = Vec::with_capacity(CHUNK_SIZE);
        let mut total_byte_count = 0i64;
        while let Ok(read_bytes) = reader.read_buf(&mut buf).await {
            total_byte_count += read_bytes as i64;
            if buf.len() > CHUNK_SIZE || read_bytes == 0 {
                let mut stolen_buf = Vec::with_capacity(CHUNK_SIZE);
                mem::swap(&mut buf, &mut stolen_buf);
                multipart
                    .put_part(PutPayload::from_bytes(Bytes::from(stolen_buf)))
                    .await?;
            }
            if read_bytes == 0 {
                break;
            }
        }
        let _result = multipart.complete().await?;

        self.size = total_byte_count;

        Ok(Model::insert(self, db)
            .await
            .context("Sqlite insert failed")?)
    }

    pub async fn get_s3_dl_link(&self, bucket: &AmazonS3) -> eyre::Result<String> {
        let url = bucket
            .signed_url_custom(
                Method::GET,
                &self.get_obj_path(),
                |url| {
                    url.query_pairs_mut().append_pair(
                        "response-content-disposition",
                        &format!("attachment; filename=\"{}\"", self.name),
                    );
                },
                Duration::from_secs(60 * 60),
            )
            .await
            .unwrap();

        Ok(url.to_string())
    }
}

pub async fn spawn_sweeper(state: Arc<AppState>) -> (JoinHandle<()>, oneshot::Sender<()>) {
    let (send, recv) = oneshot::channel();
    let thread = tokio::spawn(async move {
        let mut recv = recv;
        loop {
            let next_run = Instant::now().checked_add(Duration::from_secs(60)).unwrap();

            let mut stream = state.bucket.list(None);

            let mut threads = vec![];
            while let Some(object) = stream.next().await {
                let Ok(object) = object else {
                    error!("Sweeping error: {}", object.unwrap_err());
                    continue;
                };

                let state = state.clone();
                let t = tokio::spawn(async move {
                    let Some(Ok(slug)) = object.location.filename().map(Slug::from_str) else {
                        return None; // Skip if somehow key is invalid slug
                    };
                    let Ok(maybe_file_entry) = FileEntry::select()
                        .where_("slug = ?")
                        .bind(slug)
                        .fetch_optional(&mut *state.sqldb.write().await)
                        .await
                    else {
                        // log something here?
                        return None;
                    };
                    if maybe_file_entry.is_none() {
                        if let Err(e) = state.bucket.delete(&object.location).await {
                            error!("Failed to delete object when sweeping: {}", e);
                            None
                        } else {
                            Some(object.location)
                        }
                    } else {
                        None
                    }
                });
                threads.push(t);
            }

            let mut results = vec![];
            for thread in threads {
                if let Some(swept) = thread.await.unwrap() {
                    results.push(swept)
                };
            }

            if results.len() == 0 {
                debug!(
                    "S3 was clean, nothing swept. Next sweep in {}s",
                    next_run.duration_since(Instant::now()).as_secs_f64()
                )
            } else {
                info!(
                    "S3 was dirty, swept {:?}. Next sweep in {}s",
                    results
                        .iter()
                        .map(|x| x.filename().unwrap_or("UNKNOWN"))
                        .collect::<Vec<_>>(),
                    next_run.duration_since(Instant::now()).as_secs_f64()
                )
            }

            select! {
                _ = tokio::time::sleep_until(next_run) => {continue}
                _ = &mut recv => {break}
            };
        }
    });
    (thread, send)
}
