// ABOUTME: `harness grading-sheet` draws a blind sample from a real database; `grading-score` scores it from files alone
// ABOUTME: Pins that no verdict reaches a sheet, that a key is never overwritten, and that grades score back (carnet#854)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Blind grading-sheet export and scoring through the `harness` CLI, behind `tools-verification`.
#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use pierre_auth::key_management::KeyManager;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::{factory::Database, DatabaseProvider};
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use serde_json::Value;
use uuid::Uuid;

/// Base64 of 32 fixed bytes — the local KEK both this process and the binary use.
const MASTER_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// Open the database with the key hierarchy the binary uses.
async fn open_database(url: &str) -> Database {
    env::set_var("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY);
    let (mut key_manager, key) = KeyManager::bootstrap().unwrap();
    let mut database = Database::new(
        url,
        key.to_vec(),
        #[cfg(feature = "postgresql")]
        &PostgresPoolConfig::default(),
    )
    .await
    .unwrap();
    key_manager
        .complete_initialization(&mut database)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
}

/// A tenant with one athlete who opted in to analytics, and twelve verdicts
/// said to them: every third one contradicted, the rest unverifiable. Thirteen
/// more judged replies built from first-party-only data, so that rule
/// withholds most of the window.
async fn seed(database: &Database) -> TenantId {
    let repos = database.repositories();
    let athlete = User::new(
        format!("{}@test.local", Uuid::new_v4()),
        "hash".to_owned(),
        None,
    );
    repos.users.create(&athlete).await.unwrap();
    repos
        .users
        .update_analytics_consent(athlete.id, true)
        .await
        .unwrap();
    let tenant = Tenant::new(
        "Grading Tenant".to_owned(),
        format!("tenant-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        athlete.id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    for n in 0..25 {
        let first_party = n >= 12;
        repos
            .claim_verdicts
            .insert_claim_verdict(&InsertClaimVerdictParams {
                tenant_id: tenant.id,
                user_id: &athlete.id.to_string(),
                agent_id: None,
                conversation_id: None,
                message_id: None,
                claim_text: &if first_party {
                    format!("You rode {n} km on your provider's ride.")
                } else {
                    format!("You ran {} km yesterday.", 10 + n)
                },
                category: ClaimCategory::AthleteData,
                status: if n % 3 == 0 {
                    ClaimStatus::Contradicted
                } else {
                    ClaimStatus::Unverifiable
                },
                evidence_strength: EvidenceStrength::None,
                confidence: 0.5,
                layer_fired: VerdictLayer::AthleteData,
                explanation: Some("checker reasoning"),
                evidence_refs: None,
                transport_policy: if first_party {
                    TransportPolicy::FirstPartyOnly
                } else {
                    TransportPolicy::AnyTransport
                },
            })
            .await
            .unwrap();
    }
    tenant.id
}

fn run(args: &[&str], database_url: Option<&str>) -> Output {
    let mut command = Command::new(cli_binary());
    command.args(args);
    match database_url {
        Some(url) => command
            .env("DATABASE_URL", url)
            .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY),
        None => command
            .env_remove("DATABASE_URL")
            .env_remove("PIERRE_MASTER_ENCRYPTION_KEY"),
    };
    command.output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A grader's returned sheet: every item on `sheet` graded `false`.
fn graded_sheet(key: &Value, sheet: u64) -> String {
    let mut csv = String::from("item_id,grade,note\n");
    for item in key["items"].as_array().unwrap() {
        if item["sheets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_u64() == Some(sheet))
        {
            let _ = writeln!(csv, "{},false,", item["item_id"].as_str().unwrap());
        }
    }
    csv
}

#[tokio::test]
async fn a_drawn_sheet_hides_the_verdicts_and_its_grades_score_back() {
    let base = env::temp_dir().join(format!("pierre-cli-grading-{}", Uuid::new_v4()));
    let db_path = base.with_extension("db");
    let url = format!("sqlite:{}?mode=rwc", db_path.display());
    let database = open_database(&url).await;
    let tenant = seed(&database).await.to_string();
    let out_dir = base.join("export");
    let out = out_dir.to_str().unwrap();

    let draw = [
        "harness",
        "grading-sheet",
        "--tenant-id",
        &tenant,
        "--size",
        "10",
        "--overlap",
        "4",
        "--seed",
        "7",
        "--out-dir",
        out,
    ];
    let drawn = run(&draw, Some(&url));
    assert!(
        drawn.status.success(),
        "grading-sheet failed\nstdout: {}\nstderr: {}",
        text(&drawn.stdout),
        text(&drawn.stderr)
    );
    let stdout = text(&drawn.stdout);
    assert!(
        stdout.contains("10 claims drawn (seed 7) from the last 30 days"),
        "{stdout}"
    );
    // The first-party-only rule's share of the window, and the warning that
    // it took most of it.
    assert!(
        stdout.contains(
            "pool: 12 eligible of 25 in the window; withheld 13 first-party-only (52.0% of the window)"
        ),
        "{stdout}"
    );
    assert!(stdout.contains("warning: most of the window"), "{stdout}");

    let key_path = out_dir.join("grading-key.json");
    let key: Value = serde_json::from_str(&fs::read_to_string(&key_path).unwrap()).unwrap();
    let items = key["items"].as_array().unwrap();
    assert_eq!(items.len(), 10);
    assert_eq!(key["overlap"], 4);
    assert_eq!(key["tenant_id"], tenant.as_str());

    for n in [1, 2] {
        let sheet = fs::read_to_string(out_dir.join(format!("grading-sheet-{n}.csv"))).unwrap();
        // 4 shared items plus 3 of the 6 dealt in turn, under a header.
        assert_eq!(sheet.lines().count(), 1 + 4 + 3, "{sheet}");
        assert!(sheet.contains("You ran"), "{sheet}");
        assert!(
            !sheet.contains("You rode"),
            "a first-party-only claim reached sheet {n}"
        );
        for word in ["contradicted", "unverifiable", "athlete_data", "reasoning"] {
            assert!(!sheet.contains(word), "{word} leaked onto sheet {n}");
        }
    }

    // A second draw into the same directory would orphan the first key's
    // sheets; it is refused and the key is left as it was.
    let again = run(&draw, Some(&url));
    assert!(!again.status.success());
    assert_eq!(
        serde_json::from_str::<Value>(&fs::read_to_string(&key_path).unwrap()).unwrap(),
        key
    );

    // Both graders call every claim false: every unflagged item is a silent
    // miss, and the shared items agree.
    let unflagged = items.iter().filter(|i| i["flagged"] == false).count();
    let (ana, ben) = (base.join("ana.csv"), base.join("ben.csv"));
    fs::write(&ana, graded_sheet(&key, 1)).unwrap();
    fs::write(&ben, graded_sheet(&key, 2)).unwrap();
    let grades = |name: &str, path: &Path| format!("{name}={}", path.display());
    let scored = run(
        &[
            "harness",
            "grading-score",
            "--key",
            key_path.to_str().unwrap(),
            "--grades",
            &grades("ana", &ana),
            "--grades",
            &grades("ben", &ben),
        ],
        None,
    );
    let stdout = text(&scored.stdout);
    assert!(
        scored.status.success(),
        "grading-score failed\nstdout: {stdout}\nstderr: {}",
        text(&scored.stderr)
    );
    let silent_miss = stdout
        .lines()
        .find_map(|line| line.strip_prefix("silent-miss rate (extracted claims)"))
        .map(str::trim_start);
    assert!(
        silent_miss.is_some_and(|figures| figures.starts_with(&format!("{unflagged}/10"))),
        "{stdout}"
    );
    // The score repeats what the pool covers.
    assert!(stdout.contains("warning: most of the window"), "{stdout}");
    assert!(stdout.contains("agreement ana vs ben: 4/4"), "{stdout}");

    drop(database);
    // Best-effort cleanup of the temp files.
    drop(fs::remove_dir_all(&base));
    drop(fs::remove_file(&db_path));
}

#[test]
fn grading_score_refuses_a_grades_flag_without_a_name() {
    let dir = env::temp_dir().join(format!("pierre-cli-grading-{}", Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    let key = dir.join("key.json");
    fs::write(&key, "{}").unwrap();
    let out = run(
        &[
            "harness",
            "grading-score",
            "--key",
            key.to_str().unwrap(),
            "--grades",
            "sheet.csv",
        ],
        None,
    );
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("NAME=PATH"),
        "{}",
        text(&out.stderr)
    );
    // Best-effort cleanup of the temp files.
    drop(fs::remove_dir_all(&dir));
}
