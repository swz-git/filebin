use std::{
    collections::HashMap,
    io::{Cursor, Read},
    mem,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use axum::{body::Bytes, http::Method};
use color_eyre::eyre::{self, Context, ContextCompat};
use futures::StreamExt as _;
use object_store::{
    Attribute, AttributeValue, Attributes, GetResult, ObjectStore as _, PutMultipartOptions,
    PutPayload, aws::AmazonS3, path::Path as ObjPath, signer::Signer,
};
use ormlite::Model;
use sqlx::{SqliteConnection, Type};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, BufWriter},
    select,
    sync::oneshot,
    task::JoinHandle,
    time::Instant,
};
use tracing::{error, info, warn};

use crate::{AppState, slug::Slug};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type)]
#[sqlx(type_name = "text")] // store as TEXT in the database
pub enum Compression {
    None,
    // TODO: brotli
}

#[derive(Model, Debug)]
#[ormlite(table = "file_entries")]
pub struct FileEntry {
    pub name: String,
    #[ormlite(primary_key)]
    pub slug: Slug, // see Slug struct
    pub size: i64, // sqlite doesn't understand u32s
    pub upload_date: i64,
    pub mime: String,
    pub compression: Compression,
    #[deprecated(note = "This field will be removed in a future version")]
    pub data: Vec<u8>,
    pub admin_key: String, // uuidv4
}

pub async fn get_slugs(
    db: &mut SqliteConnection,
    slugs: impl IntoIterator<Item = Slug>,
) -> Result<Vec<FileEntry>, ormlite::SqlxError> {
    let collected_slugs: Vec<Slug> = slugs.into_iter().collect();
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
        if mem::take(&mut self.data).len() > 0 {
            warn!("Discarding data from upload, we prefer data from `reader`")
        }

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

        const CHUNK_SIZE: usize = 25 * 1024 * 1024; // 5MiB

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

    pub async fn insert(
        mut self,
        db: &mut SqliteConnection,
        bucket: &AmazonS3,
    ) -> eyre::Result<FileEntry> {
        // steal data
        let data = mem::take(&mut self.data);
        self.insert_streaming(&mut Cursor::new(data), db, bucket)
            .await
    }

    pub async fn get_s3_object(&self, bucket: &AmazonS3) -> eyre::Result<GetResult> {
        Ok(bucket.get(&self.get_obj_path()).await?)
    }

    pub async fn get_s3_dl_link(&self, bucket: &AmazonS3) -> eyre::Result<String> {
        let mut custom_queries = HashMap::<String, String>::new();
        custom_queries.insert(
            "response-content-disposition".into(),
            format!("attachment; filename=\"{}\"", self.name),
        );

        let url = bucket
            .signed_url(
                Method::GET,
                &self.get_obj_path(),
                Duration::from_secs(60 * 60),
                // Some(custom_queries), // TODO: Fix this, pretty important
            )
            .await
            .unwrap();

        Ok(url.to_string())
    }

    pub async fn data_fill(&mut self, bucket: &AmazonS3) -> eyre::Result<()> {
        self.data = self
            .get_s3_object(bucket)
            .await?
            .bytes()
            .await
            .unwrap()
            .into();
        Ok(())
    }

    pub async fn data_filled(mut self, bucket: &AmazonS3) -> eyre::Result<Self> {
        self.data_fill(bucket).await?;
        Ok(self)
    }
}

pub async fn spawn_sweeper(state: Arc<AppState>) -> (JoinHandle<()>, oneshot::Sender<()>) {
    let (send, recv) = oneshot::channel();
    let thread = tokio::spawn(async move {
        let mut recv = recv;
        loop {
            let next_run = Instant::now().checked_add(Duration::from_secs(60)).unwrap();

            'sweep: {
                let mut stream = state.bucket.list(None);

                let mut threads = vec![];
                while let Some(object) = stream.next().await {
                    let object = match object {
                        Ok(x) => x,
                        Err(e) => {
                            error!("Sweeping error: {e}");
                            continue;
                        }
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
                    info!(
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
            };

            select! {
                _ = tokio::time::sleep_until(next_run) => {continue}
                _ = &mut recv => {break}
            };
        }
    });
    (thread, send)
}
