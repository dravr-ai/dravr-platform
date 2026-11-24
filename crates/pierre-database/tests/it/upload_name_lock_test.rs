// ABOUTME: Covers the per-file upload-name lease (carnet#1701): one holder at a time for one file, released on drop
// ABOUTME: A lapsed lease is taken over; its old holder's fence fails and its release frees nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `create_test_db` opens whichever `DATABASE_URL` names, so these run on
//! `SQLite` and `PostgreSQL` alike.

use std::sync::Arc;
use std::time::Duration;

use pierre_core::models::TenantId;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::UploadedActivityFileRepository;
use pierre_test_support::db::create_test_db;
use tokio::time;
use uuid::Uuid;

const TENANT: &str = "00000000-0000-4000-8000-000000000021";

/// A digest-shaped name for file `n`.
fn digest(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

#[tokio::test]
async fn one_files_lock_has_one_holder_until_it_is_dropped() {
    let db = create_test_db().await.unwrap();
    let files: Arc<dyn UploadedActivityFileRepository> =
        Arc::clone(&db.repositories().uploaded_activity_files);
    let tenant = TenantId::parse_str(TENANT).unwrap();
    let user = Uuid::new_v4();
    let sha = digest(1);

    let first = files.lock_upload_name(&tenant, user, &sha).await.unwrap();
    assert_eq!(
        (first.tenant_id(), first.user_id(), first.file_sha256()),
        (&tenant, user, sha.as_str()),
        "bug: a lock does not name the file it holds"
    );
    let waiter = {
        let files = Arc::clone(&files);
        let sha = sha.clone();
        tokio::spawn(async move { files.lock_upload_name(&tenant, user, &sha).await.map(drop) })
    };
    time::sleep(Duration::from_millis(300)).await;
    assert!(
        !waiter.is_finished(),
        "bug: a second holder took a held file's lock"
    );
    drop(first);
    time::timeout(Duration::from_secs(10), waiter)
        .await
        .expect("bug: a dropped lock was not released")
        .unwrap()
        .unwrap();
}

/// Make every lease of the file lapse, as a holder that died leaves it.
async fn lapse(db: &Database, sha: &str) {
    let sql = "UPDATE upload_name_leases SET leased_until_ms = 0 WHERE file_sha256 = $1";
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql)
                .bind(sha)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(sql).bind(sha).execute(pg.pool()).await.unwrap();
        }
    }
}

/// A lease its holder let lapse is taken over by the next claim; the old
/// holder's fence then fails, and dropping it frees nothing of the new one.
#[tokio::test]
async fn a_lapsed_lease_is_taken_over_and_fences_out_its_old_holder() {
    let db = create_test_db().await.unwrap();
    let files: Arc<dyn UploadedActivityFileRepository> =
        Arc::clone(&db.repositories().uploaded_activity_files);
    let tenant = TenantId::parse_str(TENANT).unwrap();
    let user = Uuid::new_v4();
    let sha = digest(2);

    let stale = files.lock_upload_name(&tenant, user, &sha).await.unwrap();
    files.fence_upload_name(&stale).await.unwrap();
    lapse(&db, &sha).await;
    let taken = time::timeout(
        Duration::from_secs(5),
        files.lock_upload_name(&tenant, user, &sha),
    )
    .await
    .expect("bug: a lapsed lease was not taken over")
    .unwrap();
    assert!(
        files.fence_upload_name(&stale).await.is_err(),
        "bug: a holder whose lease was taken over still passes its fence"
    );
    files.fence_upload_name(&taken).await.unwrap();

    drop(stale);
    time::sleep(Duration::from_millis(200)).await;
    let waiter = {
        let files = Arc::clone(&files);
        let sha = sha.clone();
        tokio::spawn(async move { files.lock_upload_name(&tenant, user, &sha).await.map(drop) })
    };
    time::sleep(Duration::from_millis(300)).await;
    assert!(
        !waiter.is_finished(),
        "bug: the old holder's release freed the lease another claim took"
    );
    drop(taken);
    time::timeout(Duration::from_secs(10), waiter)
        .await
        .expect("bug: the new holder's release did not free the lease")
        .unwrap()
        .unwrap();
}
