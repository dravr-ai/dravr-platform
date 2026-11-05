// ABOUTME: GET /api/user/export, the signed-in athlete's own data as one streamed JSON document (Law 25 s.27, GDPR Article 20)
// ABOUTME: Reads one page of one table at a time, so memory stays flat however much data there is; metered per account

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `GET /api/user/export` (carnet#1346).
//!
//! The athlete downloads the personal information held about them as a single
//! JSON file, in the structured and commonly used format Law 25 s.27 and GDPR
//! Article 20 ask for. The document is
//!
//! ```json
//! { "format": "dravr-user-export", "format_version": 1, "generated_at": "…",
//!   "user_id": "…", "withheld": [{"table": "…", "reason": "…"}],
//!   "tables": { "users": [ {…} ], "cached_activities": [ … ], … },
//!   "complete": true, "failed_tables": [], "skipped_tables": [] }
//! ```
//!
//! Each key of `tables` is a database table and each row an object of its
//! columns; a column holding a JSON document is that document, not a string.
//!
//! # What is in it, and what is not
//!
//! The list lives in one place, `pierre_database::repositories::user_export`,
//! where its module documentation names what is included (the account and its
//! settings, what providers delivered, what the athlete built or said, what
//! the service recorded in their name) and what is excluded with the reason:
//! credentials (password hash, OAuth and API tokens, key and session hashes,
//! one-time codes, push tokens, the athlete's own LLM and OAuth-app secrets),
//! raw bytes of uploaded files, operational queues and cursors, operator
//! configuration, other people's records (a coach's notes about the athlete, a
//! shared room), and security logs. The document's own `withheld` list repeats
//! the excluded tables, so the athlete sees what was left out. A catalog test
//! holds that list against the live schema, so a new table cannot be forgotten
//! and a new credential-looking column cannot be exported unnoticed.
//!
//! This is not the OKF context bundle of ADR-011: that renders what the
//! assistant is shown, and this is the stored rows themselves.
//!
//! # Scope
//!
//! Every query is keyed by the signed-in athlete, and by the tenants they
//! belong to wherever the table has a tenant column; no other athlete's row is
//! reachable from this route. A credential that is not the athlete's own
//! session is refused: an API key, a messaging channel link, a token an
//! integration holds on the athlete's behalf (a marketplace client can read
//! what its scopes allow, never carry the whole file away), and an operator's
//! impersonation token (support may see what it needs to help, never carry the
//! whole file away either).
//!
//! # Bounds
//!
//! The response is streamed. A task reads one page of one table (a few rows to
//! a few hundred, by how large a row of that table is), writes it into a
//! channel two chunks deep, and reads the next page only when the client has
//! drained the channel, so memory is a page plus two chunks whatever the size
//! of the athlete's history. Pages run in primary-key order, so a row updated
//! while the file is being written is neither dropped nor repeated; a row added
//! meanwhile may or may not appear, since the file is not one snapshot. A client that disconnects stops the task. A file
//! past [`MAX_EXPORT_BYTES`] stops there and says so (`complete: false`,
//! `skipped_tables`) rather than growing without end, and so does one still
//! being written at [`MAX_EXPORT_DURATION`], so the load balancer's limit on a
//! request never cuts the document in two. Each account may export
//! [`MAX_EXPORTS_PER_WINDOW`] times an hour, and the server runs at most
//! [`MAX_CONCURRENT_EXPORTS`] at once.

use std::io;
use std::mem;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, RETRY_AFTER},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::{mpsc, Semaphore};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, field::Empty, info, warn, Span};
use uuid::Uuid;

use pierre_auth::auth::AuthMethod;
use pierre_auth::oauth2_server::first_party::is_first_party;
use pierre_cache::{CacheKey, CacheResource};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    UserExportRepository, UserExportTable, USER_EXPORT_EXCLUDED_TABLES, USER_EXPORT_TABLES,
};

use crate::AuthRoutesContext;

/// How many exports one account may start in [`RATE_WINDOW`].
const MAX_EXPORTS_PER_WINDOW: u64 = 3;
/// The window [`MAX_EXPORTS_PER_WINDOW`] is counted in.
const RATE_WINDOW: Duration = Duration::from_hours(1);
/// How many exports the server writes at once.
const MAX_CONCURRENT_EXPORTS: usize = 4;
/// The size past which the file stops growing.
const MAX_EXPORT_BYTES: usize = 512 * 1024 * 1024;
/// How long a file is written before it stops and says so. The frontend's
/// Cloud Run service ends every request at 600 s (`infra/environments/dev/main.tf`);
/// stopping first leaves the athlete a complete document that says it is partial,
/// never a truncated one.
const MAX_EXPORT_DURATION: Duration = Duration::from_secs(540);
/// The size a chunk reaches before it is sent.
const CHUNK_BYTES: usize = 64 * 1024;
/// How many chunks wait for the client to read before the task pauses.
const CHANNEL_DEPTH: usize = 2;
/// Seconds a refusal for lack of a free export slot asks the client to wait.
const BUSY_RETRY_SECS: u64 = 30;
/// The cache namespace of the per-account window.
const RATE_NAMESPACE: &str = "_user_export_rate";

/// Export slots; a permit is held for as long as its export is being written.
static EXPORT_SLOTS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_EXPORTS)));

/// Where a written chunk goes. The route sends it down the response body;
/// tests collect it.
trait ChunkSink {
    /// Hand over a chunk; `false` when nobody is reading any more.
    async fn send(&mut self, chunk: Bytes) -> bool;
}

impl ChunkSink for mpsc::Sender<Result<Bytes, io::Error>> {
    async fn send(&mut self, chunk: Bytes) -> bool {
        Self::send(self, Ok(chunk)).await.is_ok()
    }
}

/// The size limits a write runs under.
#[derive(Debug, Clone, Copy)]
struct ExportLimits {
    /// The file stops growing past this many bytes.
    max_bytes: usize,
    /// The file stops growing this long after it started.
    max_duration: Duration,
    /// A chunk is sent once it holds this many bytes.
    chunk_bytes: usize,
}

/// What a write did, for the log and the closing fields of the document.
#[derive(Debug, Default)]
struct ExportSummary {
    /// Rows written.
    rows: u64,
    /// Bytes written.
    bytes: usize,
    /// Tables whose read failed part-way; the file holds the rows before it.
    failed_tables: Vec<&'static str>,
    /// Tables not read, or not read to their end, because the file reached its
    /// size or time limit.
    skipped_tables: Vec<&'static str>,
    /// The client went away before the end.
    disconnected: bool,
}

impl ExportSummary {
    /// Name `table` among the failed ones, once however many of its rows failed.
    fn mark_failed(&mut self, table: &'static str) {
        if !self.failed_tables.contains(&table) {
            self.failed_tables.push(table);
        }
    }
}

/// Buffers the document and sends it in chunks.
struct Writer<'sink, S: ChunkSink> {
    sink: &'sink mut S,
    buffer: Vec<u8>,
    sent: usize,
    limits: ExportLimits,
    started: Instant,
    disconnected: bool,
}

impl<S: ChunkSink> Writer<'_, S> {
    /// Whether the file has reached its size or time limit.
    fn out_of_room(&self) -> bool {
        self.written() >= self.limits.max_bytes
            || self.started.elapsed() >= self.limits.max_duration
    }

    /// Append `bytes`, sending a chunk when the buffer is full.
    async fn put(&mut self, bytes: &[u8]) {
        if self.disconnected {
            return;
        }
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= self.limits.chunk_bytes {
            self.flush().await;
        }
    }

    /// Send what is buffered.
    async fn flush(&mut self) {
        if self.buffer.is_empty() || self.disconnected {
            return;
        }
        self.sent += self.buffer.len();
        let chunk = Bytes::from(mem::take(&mut self.buffer));
        if !self.sink.send(chunk).await {
            self.disconnected = true;
        }
    }

    /// Bytes written so far, sent or still buffered.
    fn written(&self) -> usize {
        self.sent + self.buffer.len()
    }
}

/// Write the whole document for `user_id` into `sink`.
///
/// A table whose read fails, or that does not fit under the size limit, is
/// recorded and the document is closed properly, so what the athlete holds is
/// always valid JSON that says whether it is complete.
async fn write_export<S: ChunkSink>(
    repository: &dyn UserExportRepository,
    user_id: Uuid,
    sink: &mut S,
    limits: ExportLimits,
) -> ExportSummary {
    let mut writer = Writer {
        sink,
        buffer: Vec::with_capacity(limits.chunk_bytes * 2),
        sent: 0,
        limits,
        started: Instant::now(),
        disconnected: false,
    };
    let withheld: Vec<Value> = USER_EXPORT_EXCLUDED_TABLES
        .iter()
        .map(|(table, reason)| json!({ "table": table, "reason": reason }))
        .collect();
    let head = json!({
        "format": "dravr-user-export",
        "format_version": 1,
        "generated_at": Utc::now().to_rfc3339(),
        "user_id": user_id,
        "withheld": withheld,
    });
    // The head object without its closing brace, so `tables` follows in it.
    let mut head_text = head.to_string();
    head_text.pop();
    writer.put(head_text.as_bytes()).await;
    writer.put(br#","tables":{"#).await;

    let mut summary = ExportSummary::default();
    for (index, table) in USER_EXPORT_TABLES.iter().enumerate() {
        if writer.disconnected {
            break;
        }
        if writer.out_of_room() {
            summary.skipped_tables.extend(
                USER_EXPORT_TABLES[index..]
                    .iter()
                    .map(|remaining| remaining.name),
            );
            break;
        }
        if index > 0 {
            writer.put(b",").await;
        }
        write_table(repository, table, user_id, &mut writer, &mut summary).await;
    }

    let tail = json!({
        "complete": summary.failed_tables.is_empty()
            && summary.skipped_tables.is_empty()
            && !writer.disconnected,
        "failed_tables": summary.failed_tables,
        "skipped_tables": summary.skipped_tables,
    });
    let mut tail_text = tail.to_string();
    // `}` closes `tables`; the tail's own fields continue the document.
    tail_text.replace_range(0..1, "},");
    writer.put(tail_text.as_bytes()).await;
    writer.flush().await;

    summary.bytes = writer.written();
    summary.disconnected = writer.disconnected;
    summary
}

/// Write `"name":[row,…]`, a page at a time.
async fn write_table<S: ChunkSink>(
    repository: &dyn UserExportRepository,
    table: &'static UserExportTable,
    user_id: Uuid,
    writer: &mut Writer<'_, S>,
    summary: &mut ExportSummary,
) {
    writer.put(format!("\"{}\":[", table.name).as_bytes()).await;
    let mut after = String::new();
    let mut first = true;
    loop {
        if writer.disconnected {
            break;
        }
        let page = match repository.export_page(table, user_id, &after).await {
            Ok(page) => page,
            Err(error) => {
                warn!(table = table.name, error = %error, "User export could not read a table");
                summary.mark_failed(table.name);
                break;
            }
        };
        let read = i64::try_from(page.rows.len()).unwrap_or(i64::MAX);
        for row in &page.rows {
            if writer.out_of_room() {
                summary.skipped_tables.push(table.name);
                writer.put(b"]").await;
                return;
            }
            if !first {
                writer.put(b",").await;
            }
            first = false;
            write_row(table, row, writer, summary).await;
        }
        match page.last_key {
            Some(last_key) if read >= table.page_rows => after = last_key,
            _ => break,
        }
    }
    writer.put(b"]").await;
}

/// Write one row; a row that cannot be written is `null`, and its table is named.
async fn write_row<S: ChunkSink>(
    table: &'static UserExportTable,
    row: &Value,
    writer: &mut Writer<'_, S>,
    summary: &mut ExportSummary,
) {
    match serde_json::to_vec(row) {
        Ok(text) => writer.put(&text).await,
        Err(error) => {
            warn!(table = table.name, error = %error, "User export could not write a row");
            summary.mark_failed(table.name);
            writer.put(b"null").await;
        }
    }
    summary.rows += 1;
}

/// The signed-in athlete's id, refusing every credential that is not their own
/// first-party session.
async fn session_user_id(resources: &AuthRoutesContext, headers: &HeaderMap) -> AppResult<Uuid> {
    let auth = resources
        .auth_middleware
        .authenticate_request_with_headers(headers)
        .await?;
    let first_party_client = auth.client_id.as_deref().is_none_or(is_first_party);
    let own_session = matches!(
        auth.auth_method,
        AuthMethod::JwtToken {
            impersonation: None,
            ..
        }
    );
    if !own_session || !first_party_client {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Your data can only be exported from your own signed-in session",
        ));
    }
    Span::current().record("user_id", auth.user_id.to_string());
    Ok(auth.user_id)
}

/// Count one export in the account's hourly window; `Some(seconds)` when the
/// account has used its exports.
async fn meter_export(resources: &AuthRoutesContext, user_id: Uuid) -> AppResult<Option<u64>> {
    let key = CacheKey::new(
        TenantId::nil(),
        Uuid::nil(),
        RATE_NAMESPACE.to_owned(),
        CacheResource::Custom(format!("user:{user_id}")),
    );
    let counted = resources
        .cache
        .count_in_window(&key, RATE_WINDOW)
        .await
        .map_err(|error| {
            warn!(user_id = %user_id, error = %error, "User export limiter could not count the request");
            AppError::resource_unavailable("Data export is temporarily unavailable; retry later")
        })?;
    if counted.hits > MAX_EXPORTS_PER_WINDOW {
        return Ok(Some(counted.resets_in.as_secs().max(1)));
    }
    Ok(None)
}

/// A JSON refusal the clients localize by its `error` code.
fn refusal(status: StatusCode, error: &str, message: &str, retry_after: u64) -> Response {
    let mut response = (
        status,
        Json(json!({ "error": error, "message": message, "retry_after_seconds": retry_after })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from(retry_after));
    response
}

/// `GET /api/user/export` — stream the signed-in athlete's data as JSON.
///
/// Refuses with 429 once the account has used its exports for the hour, and
/// with 503 while the server is writing as many exports as it allows.
///
/// # Errors
///
/// Returns an authentication error without a session, a permission error for
/// an API key, a channel link or a third-party token, and a
/// resource-unavailable error when the limiter cannot count the request.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "user_export", user_id = Empty)
)]
pub(crate) async fn handle_user_export(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let user_id = session_user_id(&resources, &headers).await?;
    if let Some(retry_after) = meter_export(&resources, user_id).await? {
        info!(user_id = %user_id, "User export refused: the hour's exports are used");
        return Ok(refusal(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_exports",
            "You have exported your data several times this hour; retry later",
            retry_after,
        ));
    }
    let Ok(permit) = Arc::clone(&*EXPORT_SLOTS).try_acquire_owned() else {
        return Ok(refusal(
            StatusCode::SERVICE_UNAVAILABLE,
            "export_busy",
            "Exports are busy; retry in a moment",
            BUSY_RETRY_SECS,
        ));
    };

    let repos = Arc::clone(&resources.repos);
    let (sender, receiver) = mpsc::channel::<Result<Bytes, io::Error>>(CHANNEL_DEPTH);
    let on_panic = sender.clone();
    // The task owns the slot until the document is written or the client is
    // gone, and reports its own outcome.
    let writing = tokio::spawn(async move {
        let mut sink = sender;
        let summary = write_export(
            repos.user_export.as_ref(),
            user_id,
            &mut sink,
            ExportLimits {
                max_bytes: MAX_EXPORT_BYTES,
                max_duration: MAX_EXPORT_DURATION,
                chunk_bytes: CHUNK_BYTES,
            },
        )
        .await;
        info!(
            user_id = %user_id,
            rows = summary.rows,
            bytes = summary.bytes,
            failed_tables = summary.failed_tables.len(),
            skipped_tables = summary.skipped_tables.len(),
            disconnected = summary.disconnected,
            "User data export finished"
        );
        drop(permit);
    });
    // A panic would otherwise end the body cleanly, so a cut file would read as
    // a finished one: it ends the body with an error instead.
    drop(tokio::spawn(async move {
        if let Err(failure) = writing.await {
            error!(user_id = %user_id, error = %failure, "User data export task failed");
            on_panic
                .send(Err(io::Error::other("the export stopped")))
                .await
                .ok();
        }
    }));

    let filename = format!(
        "attachment; filename=\"dravr-export-{}.json\"",
        Utc::now().format("%Y-%m-%d")
    );
    let mut response = Response::new(Body::from_stream(ReceiverStream::new(receiver)));
    let response_headers = response.headers_mut();
    response_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&filename) {
        response_headers.insert(CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use pierre_database::repositories::UserExportPage;
    use tokio::task;

    use super::*;

    /// Serves `rows_per_table` rows of `row_bytes` characters for each table,
    /// failing on `failing`, and records the largest page it handed out.
    struct FakeRepository {
        rows_per_table: usize,
        row_bytes: usize,
        failing: Option<&'static str>,
    }

    #[async_trait]
    impl UserExportRepository for FakeRepository {
        async fn export_page(
            &self,
            table: &UserExportTable,
            _user_id: Uuid,
            after: &str,
        ) -> AppResult<UserExportPage> {
            if self.failing == Some(table.name) {
                return Err(AppError::database("boom"));
            }
            let start = if after.is_empty() {
                0
            } else {
                after.parse::<usize>().unwrap() + 1
            };
            let end = (start + usize::try_from(table.page_rows).unwrap()).min(self.rows_per_table);
            Ok(UserExportPage {
                rows: (start..end)
                    .map(|n| json!({ "n": n, "pad": "x".repeat(self.row_bytes) }))
                    .collect(),
                last_key: (end > start).then(|| (end - 1).to_string()),
            })
        }
    }

    /// Collects chunks, optionally going away after `leave_after` of them.
    struct Collector {
        chunks: Vec<Bytes>,
        leave_after: Option<usize>,
    }

    impl ChunkSink for Collector {
        async fn send(&mut self, chunk: Bytes) -> bool {
            // As the channel does, give the task's turn away between chunks.
            task::yield_now().await;
            if self.leave_after.is_some_and(|n| self.chunks.len() >= n) {
                return false;
            }
            self.chunks.push(chunk);
            true
        }
    }

    fn collector() -> Collector {
        Collector {
            chunks: Vec::new(),
            leave_after: None,
        }
    }

    fn document(sink: &Collector) -> Value {
        let bytes: Vec<u8> = sink.chunks.iter().flat_map(|c| c.to_vec()).collect();
        serde_json::from_slice(&bytes).expect("the export is valid JSON whatever happened")
    }

    const ROOMY: ExportLimits = ExportLimits {
        max_bytes: usize::MAX,
        max_duration: Duration::from_secs(3600),
        chunk_bytes: 1024,
    };

    #[tokio::test]
    async fn the_document_is_valid_json_with_every_table() {
        let repository = FakeRepository {
            rows_per_table: 7,
            row_bytes: 10,
            failing: None,
        };
        let mut sink = collector();
        let summary = write_export(&repository, Uuid::new_v4(), &mut sink, ROOMY).await;

        let doc = document(&sink);
        assert_eq!(doc["format"], "dravr-user-export");
        assert_eq!(doc["complete"], true);
        let tables = doc["tables"].as_object().unwrap();
        assert_eq!(
            tables.len(),
            USER_EXPORT_TABLES.len(),
            "every table has its key"
        );
        assert_eq!(tables["users"].as_array().unwrap().len(), 7);
        assert_eq!(summary.rows, 7 * USER_EXPORT_TABLES.len() as u64);
        assert_eq!(
            doc["withheld"].as_array().unwrap().len(),
            USER_EXPORT_EXCLUDED_TABLES.len(),
            "the document names what it leaves out"
        );
    }

    #[tokio::test]
    async fn a_file_out_of_time_stops_as_a_valid_document_that_says_so() {
        let repository = FakeRepository {
            rows_per_table: 3,
            row_bytes: 10,
            failing: None,
        };
        let mut sink = collector();
        let limits = ExportLimits {
            max_duration: Duration::ZERO,
            ..ROOMY
        };
        write_export(&repository, Uuid::new_v4(), &mut sink, limits).await;

        let doc = document(&sink);
        assert_eq!(doc["complete"], false);
        assert_eq!(
            doc["skipped_tables"].as_array().unwrap().len(),
            USER_EXPORT_TABLES.len(),
            "it names every table it did not reach"
        );
    }

    #[tokio::test]
    async fn a_failing_table_is_named_and_the_document_stays_valid() {
        let repository = FakeRepository {
            rows_per_table: 3,
            row_bytes: 10,
            failing: Some("recipes"),
        };
        let mut sink = collector();
        write_export(&repository, Uuid::new_v4(), &mut sink, ROOMY).await;

        let doc = document(&sink);
        assert_eq!(doc["complete"], false);
        assert_eq!(doc["failed_tables"], json!(["recipes"]));
        assert_eq!(doc["tables"]["training_plans"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_table_is_named_once_however_many_of_its_rows_failed() {
        let mut summary = ExportSummary::default();
        summary.mark_failed("recipes");
        summary.mark_failed("recipes");
        summary.mark_failed("goals");
        assert_eq!(summary.failed_tables, ["recipes", "goals"]);
    }

    #[tokio::test]
    async fn a_file_past_its_limit_stops_and_says_what_it_skipped() {
        let repository = FakeRepository {
            rows_per_table: 50,
            row_bytes: 200,
            failing: None,
        };
        let mut sink = collector();
        let limits = ExportLimits {
            max_bytes: 20_000,
            max_duration: Duration::from_secs(3600),
            chunk_bytes: 1024,
        };
        let summary = write_export(&repository, Uuid::new_v4(), &mut sink, limits).await;

        let doc = document(&sink);
        assert_eq!(doc["complete"], false);
        assert_ne!(doc["skipped_tables"], json!([]));
        // The limit is on rows written; the closing fields come after it.
        assert!(
            summary.bytes < 20_000 + 8 * 1024,
            "the file kept growing: {}",
            summary.bytes
        );
    }

    #[tokio::test]
    async fn chunks_stay_small_however_many_rows_there_are() {
        let repository = FakeRepository {
            rows_per_table: 300,
            row_bytes: 100,
            failing: None,
        };
        let mut sink = collector();
        write_export(&repository, Uuid::new_v4(), &mut sink, ROOMY).await;

        let largest = sink.chunks.iter().map(Bytes::len).max().unwrap();
        assert!(
            sink.chunks.len() > 100,
            "the document went out in many pieces"
        );
        assert!(
            largest < 16 * 1024,
            "no chunk is more than a chunk limit and one row past it: {largest}"
        );
    }

    #[tokio::test]
    async fn a_client_that_leaves_stops_the_reads() {
        let repository = FakeRepository {
            rows_per_table: 300,
            row_bytes: 100,
            failing: None,
        };
        let mut sink = Collector {
            chunks: Vec::new(),
            leave_after: Some(3),
        };
        let summary = write_export(&repository, Uuid::new_v4(), &mut sink, ROOMY).await;

        assert!(summary.disconnected);
        assert_eq!(
            sink.chunks.len(),
            3,
            "nothing is sent after the client left"
        );
        assert!(
            summary.rows < 300 * USER_EXPORT_TABLES.len() as u64 / 10,
            "the task stopped reading: {} rows",
            summary.rows
        );
    }
}
