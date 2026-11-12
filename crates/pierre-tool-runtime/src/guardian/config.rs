// ABOUTME: Guardian runtime configuration — the stored per-turn caps and the hot-reloadable registry
// ABOUTME: system_settings.guardian_config resolves into the Arc<Guardian> every dispatch reads; each instance re-reads the row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Guardian Config Registry
//!
//! Holds the effective [`Guardian`] snapshot every dispatch point reads
//! through `ToolRuntime::guardian()`. Bootstrapped at server startup from
//! `system_settings.guardian_config`; if the row is absent or invalid, the
//! snapshot resolves from compiled-in defaults. `PUT /admin/settings/guardian`
//! validates and persists a new [`GuardianConfigDocument`], then calls
//! [`GuardianConfigRegistry::install`] so subsequent dispatches see the new
//! caps without a server restart.
//!
//! Resolution is per field: the compiled-in default, or the stored document's
//! value when it sets one.
//!
//! [`GuardianConfigRegistry::install`] only swaps the local process, and the
//! deployment runs several instances with no session affinity, so an admin
//! save lands on one of them. Every instance therefore re-reads the row on an
//! interval ([`GuardianConfigRegistry::reload_from_database`]) and converges on
//! it. A read failure or a row that does not parse keeps the policy in force:
//! a poll never reverts an instance to defaults because of a fault.
//!
//! `max_writes_per_turn: 0`, which would deny every write-tool dispatch, resolves
//! away at load rather than taking effect, logged at `error!` and reported as
//! degraded.
//!
//! A row written by an earlier binary may carry `schema_version` and `mode`.
//! Both are read and ignored ([`RETIRED_FIELDS`]): the gate always enforces, and
//! the document is read tolerantly instead of by version.

use std::sync::Arc;

use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::factory::Database;
use serde::{Deserialize, Serialize};
use serde_json::{Error as JsonError, Map, Value};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::policy::GuardianPolicy;
use super::Guardian;

/// `system_settings` key holding the persisted [`GuardianConfigDocument`].
pub const GUARDIAN_CONFIG_SETTING_KEY: &str = "guardian_config";

/// The fields [`GuardianConfigDocument`] reads.
const DOCUMENT_FIELDS: [&str; 2] = ["max_destructive_per_turn", "max_writes_per_turn"];

/// Keys a row written by an earlier binary carries, which this one ignores.
///
/// `schema_version` versioned the document, and `mode` chose whether a denial
/// blocked. The gate now always blocks, so a stored `"observe"` enforces.
const RETIRED_FIELDS: [&str; 2] = ["schema_version", "mode"];

/// Top-level document persisted under [`GUARDIAN_CONFIG_SETTING_KEY`].
///
/// Every field is optional: an unset field follows the compiled-in default, so
/// a document written today does not freeze today's defaults. Unknown keys are
/// ignored on read, so a row an earlier binary wrote still loads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardianConfigDocument {
    /// Override for [`GuardianPolicy::max_destructive_per_turn`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_destructive_per_turn: Option<u16>,
    /// Override for [`GuardianPolicy::max_writes_per_turn`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_writes_per_turn: Option<u16>,
}

/// Reject a document before it reaches storage.
///
/// The field types already constrain the value space (budgets are `u16`), so
/// the one check is the semantic foot-gun: `max_writes_per_turn: 0` would block
/// every write-tool dispatch platform-wide, which is an outage switch, not a
/// budget.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when the write budget is zero.
pub fn validate_document(doc: &GuardianConfigDocument) -> AppResult<()> {
    (doc.max_writes_per_turn != Some(0)).ok_or_else(|| {
        AppError::invalid_input(
            "max_writes_per_turn must be >= 1 (0 would deny every write-tool dispatch)",
        )
    })
}

/// A stored `guardian_config` row, read as this binary understands it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedGuardianDocument {
    /// The fields this binary reads.
    pub document: GuardianConfigDocument,
    /// Keys in the row this binary neither reads nor retired, which a save drops.
    pub unknown_fields: Vec<String>,
    /// Retired keys the row still carries ([`RETIRED_FIELDS`]).
    pub retired_fields: Vec<String>,
}

/// Parse a stored row without silently losing what it holds.
///
/// Only a JSON object is a document: serde would otherwise read `[]` as a
/// struct of defaults and call a corrupt row valid.
///
/// # Errors
///
/// Returns the parse error when the row is not an object, or a field this
/// binary reads holds a value of the wrong type.
pub fn parse_persisted_document(raw: &str) -> Result<PersistedGuardianDocument, JsonError> {
    let fields: Map<String, Value> = serde_json::from_str(raw)?;
    let (mut unknown_fields, mut retired_fields) = (Vec::new(), Vec::new());
    for key in fields.keys() {
        if RETIRED_FIELDS.contains(&key.as_str()) {
            retired_fields.push(key.clone());
        } else if !DOCUMENT_FIELDS.contains(&key.as_str()) {
            unknown_fields.push(key.clone());
        }
    }
    unknown_fields.sort();
    retired_fields.sort();
    let document = serde_json::from_value(Value::Object(fields))?;
    Ok(PersistedGuardianDocument {
        document,
        unknown_fields,
        retired_fields,
    })
}

/// Snapshot held by the registry: the source document plus the pre-resolved
/// [`Guardian`] so a per-dispatch read is one `Arc` clone.
#[derive(Debug, Clone)]
pub struct GuardianConfigSnapshot {
    /// Document as persisted.
    pub document: Arc<GuardianConfigDocument>,
    /// Effective guard: defaults with the document over them, pre-built per snapshot.
    pub guardian: Arc<Guardian>,
    /// When the snapshot was installed.
    pub loaded_at: DateTime<Utc>,
    /// Why this snapshot does not reflect the stored row: the row could not be
    /// read or parsed, so the policy in force is the one held before that, or
    /// the defaults at boot.
    pub load_failure: Option<String>,
    /// Document fields the resolution refused, each saying why.
    pub refused_fields: Vec<String>,
}

impl GuardianConfigSnapshot {
    /// Why this snapshot is not simply the stored row in force, when it is not.
    #[must_use]
    pub fn degraded_reason(&self) -> Option<String> {
        let reasons: Vec<&str> = self
            .load_failure
            .iter()
            .chain(&self.refused_fields)
            .map(String::as_str)
            .collect();
        (!reasons.is_empty()).then(|| reasons.join("; "))
    }
}

/// What one [`GuardianConfigRegistry::reload_from_database`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardianReloadOutcome {
    /// The row matches the snapshot.
    Unchanged,
    /// The snapshot now reflects the row.
    Installed,
    /// The row could not be read; the snapshot stays.
    KeptOnReadFailure,
    /// The row does not parse; the snapshot's policy stays, marked degraded.
    KeptOnInvalidRow,
}

/// Thread-safe registry holding the current Guardian config snapshot.
///
/// Reads are lock-free and return an `Arc` clone, so they are safe on the
/// hot dispatch path. A write is an admin PUT ([`Self::install`]) or a poll
/// that found the row changed.
#[derive(Debug)]
pub struct GuardianConfigRegistry {
    snapshot: ArcSwap<GuardianConfigSnapshot>,
    instance_nonce: Uuid,
}

impl GuardianConfigRegistry {
    /// Build a registry with no persisted document: the compiled-in defaults.
    #[must_use]
    pub fn bootstrap() -> Self {
        Self::with_document(GuardianConfigDocument::default())
    }

    /// Build a registry over `document`, without a database.
    #[must_use]
    pub fn with_document(document: GuardianConfigDocument) -> Self {
        Self {
            snapshot: ArcSwap::from_pointee(build_snapshot(document)),
            instance_nonce: Uuid::new_v4(),
        }
    }

    /// A registry on the defaults because the row could not be used at boot.
    fn degraded_defaults(reason: String) -> Self {
        let registry = Self::bootstrap();
        registry.snapshot.store(Arc::new(GuardianConfigSnapshot {
            load_failure: Some(reason),
            ..(*registry.current()).clone()
        }));
        registry
    }

    /// Load the document from `system_settings.guardian_config` at startup.
    ///
    /// On any read or parse failure, falls back to the compiled-in defaults so
    /// the server boots, marks the snapshot degraded so the admin GET says so,
    /// and logs at `error!`. The first poll that reads a good row repairs it.
    pub async fn from_database(db: &Database) -> Self {
        match db.get_system_setting(GUARDIAN_CONFIG_SETTING_KEY).await {
            Ok(Some(row)) => Self::from_persisted_value(&row.value),
            Ok(None) => {
                info!("guardian_config row absent in system_settings; using compiled-in defaults");
                Self::bootstrap()
            }
            Err(e) => {
                error!(
                    error = %e,
                    "guardian_config DB load failed; falling back to compiled-in defaults"
                );
                Self::degraded_defaults(
                    "the row could not be read at boot; running the compiled-in defaults".into(),
                )
            }
        }
    }

    fn from_persisted_value(json: &str) -> Self {
        match parse_persisted_document(json) {
            Ok(persisted) => {
                log_stored_semantics(&persisted);
                Self::with_document(persisted.document)
            }
            Err(e) => {
                log_invalid_row(&e);
                Self::degraded_defaults(format!(
                    "the row does not parse ({e}); running the compiled-in defaults"
                ))
            }
        }
    }

    /// Re-read the row and install it when it differs from the snapshot: one
    /// poll tick.
    ///
    /// A read failure keeps the snapshot and logs at `warn!`. A row that does
    /// not parse keeps the policy in force, marks it degraded and logs at
    /// `error!` on every tick, so an alert stays lit until the row is fixed.
    /// Neither path reverts to defaults; only a read that finds no row does,
    /// since that is what an absent row means at boot too.
    ///
    /// The swap is against the snapshot held before the read, so a tick that
    /// read the row just before an admin PUT landed on this instance cannot put
    /// the older document back: it loses the swap, and the next tick compares
    /// against what the PUT installed.
    pub async fn reload_from_database(&self, db: &Database) -> GuardianReloadOutcome {
        let before = self.snapshot.load_full();
        let row = match db.get_system_setting(GUARDIAN_CONFIG_SETTING_KEY).await {
            Ok(row) => row,
            Err(e) => {
                warn!(
                    error = %e,
                    "guardian_config poll could not read the row; keeping the current policy"
                );
                return GuardianReloadOutcome::KeptOnReadFailure;
            }
        };
        let Some(row) = row else {
            return self.settle(&before, GuardianConfigDocument::default());
        };
        match parse_persisted_document(&row.value) {
            Ok(persisted) => {
                if before.load_failure.is_some() || *before.document != persisted.document {
                    log_stored_semantics(&persisted);
                }
                self.settle(&before, persisted.document)
            }
            Err(e) => {
                log_invalid_row(&e);
                self.keep_marked_invalid(&before, &e);
                GuardianReloadOutcome::KeptOnInvalidRow
            }
        }
    }

    /// Install `document` unless the snapshot already holds it, against the
    /// snapshot the caller read before going to the database.
    fn settle(
        &self,
        before: &Arc<GuardianConfigSnapshot>,
        document: GuardianConfigDocument,
    ) -> GuardianReloadOutcome {
        if before.load_failure.is_none() && *before.document == document {
            return GuardianReloadOutcome::Unchanged;
        }
        let next = Arc::new(build_snapshot(document));
        let prior = self.snapshot.compare_and_swap(before, Arc::clone(&next));
        if !Arc::ptr_eq(&*prior, before) {
            return GuardianReloadOutcome::Unchanged;
        }
        let policy = next.guardian.policy();
        info!(
            max_destructive_per_turn = policy.max_destructive_per_turn,
            max_writes_per_turn = policy.max_writes_per_turn,
            "guardian_config poll installed a changed row"
        );
        GuardianReloadOutcome::Installed
    }

    /// Say on the snapshot that the row is unusable, keeping its policy.
    fn keep_marked_invalid(&self, before: &Arc<GuardianConfigSnapshot>, cause: &JsonError) {
        let reason = format!("the row does not parse ({cause}); keeping the last good policy");
        if before.load_failure.as_deref() == Some(reason.as_str()) {
            return;
        }
        let next = GuardianConfigSnapshot {
            load_failure: Some(reason),
            ..(**before).clone()
        };
        // Losing this swap to a concurrent install is fine: the next tick marks
        // whatever that install left in place.
        drop(self.snapshot.compare_and_swap(before, Arc::new(next)));
    }

    /// Cheap clone of the entire current snapshot (admin GET / diagnostics).
    #[must_use]
    pub fn current(&self) -> Arc<GuardianConfigSnapshot> {
        self.snapshot.load_full()
    }

    /// The effective guard for the current snapshot. One atomic `Arc` clone —
    /// this is the per-dispatch read behind `ToolRuntime::guardian()`.
    #[must_use]
    pub fn current_guardian(&self) -> Arc<Guardian> {
        Arc::clone(&self.snapshot.load().guardian)
    }

    /// Unique to this registry, and so to this process: the part of an instance
    /// id two instances of one revision do not share.
    #[must_use]
    pub const fn instance_nonce(&self) -> Uuid {
        self.instance_nonce
    }

    /// Replace the current snapshot with a fresh document.
    ///
    /// Called by the admin PUT handler after [`validate_document`] has
    /// succeeded and the row has been persisted (write-then-install, so a DB
    /// failure leaves both stores on the previous values).
    pub fn install(&self, document: GuardianConfigDocument) {
        self.snapshot.store(Arc::new(build_snapshot(document)));
    }
}

fn build_snapshot(document: GuardianConfigDocument) -> GuardianConfigSnapshot {
    let (policy, refused_fields) = resolve(&document);
    GuardianConfigSnapshot {
        document: Arc::new(document),
        guardian: Arc::new(Guardian::new(policy)),
        loaded_at: Utc::now(),
        load_failure: None,
        refused_fields,
    }
}

/// The line an alert matches for a row that does not parse, at boot and on
/// every poll tick: one literal so the two paths cannot drift apart.
fn log_invalid_row(cause: &JsonError) {
    error!(error = %cause, "guardian_config row is invalid");
}

/// Log what a row holds that this binary will not honor: never silently.
fn log_stored_semantics(persisted: &PersistedGuardianDocument) {
    if !persisted.unknown_fields.is_empty() {
        error!(
            unknown = ?persisted.unknown_fields,
            "guardian_config row carries fields this binary does not read"
        );
    }
    if !persisted.retired_fields.is_empty() {
        info!(
            retired = ?persisted.retired_fields,
            "guardian_config row carries retired fields; ignoring them, the gate always enforces"
        );
    }
    info!("guardian_config loaded from system_settings");
}

/// The document's write budget, or `None` when it asks for zero.
fn usable_write_budget(doc: &GuardianConfigDocument, refused: &mut Vec<String>) -> Option<u16> {
    if doc.max_writes_per_turn == Some(0) {
        error!(
            "guardian_config max_writes_per_turn is 0, which would deny every write-tool \
             dispatch; resolving the field to its default"
        );
        refused.push("max_writes_per_turn=0 is refused and resolves to the default".to_owned());
        return None;
    }
    doc.max_writes_per_turn
}

/// A destructive budget of 0 stays legal, since refusing every irreversible tool
/// is a posture someone can mean, and it is also what a slip looks like. So it is
/// never refused and never quiet: logged each time a snapshot is built, which is
/// at boot, on a save and on a poll that found a change.
fn warn_on_zero_destructive_budget(budget: u16) {
    if budget == 0 {
        warn!(
            "guardian max_destructive_per_turn is 0: every irreversible tool is denied on every \
             turn"
        );
    }
}

fn resolve(doc: &GuardianConfigDocument) -> (GuardianPolicy, Vec<String>) {
    let defaults = GuardianPolicy::default();
    let mut refused = Vec::new();
    let max_writes_per_turn =
        usable_write_budget(doc, &mut refused).unwrap_or(defaults.max_writes_per_turn);
    let max_destructive_per_turn = doc
        .max_destructive_per_turn
        .unwrap_or(defaults.max_destructive_per_turn);
    warn_on_zero_destructive_budget(max_destructive_per_turn);
    (
        GuardianPolicy {
            max_destructive_per_turn,
            max_writes_per_turn,
        },
        refused,
    )
}

#[cfg(test)]
mod tests {
    use std::io::{Result as IoResult, Write};
    use std::sync::Mutex;

    use tracing::subscriber::with_default;
    use tracing_subscriber::fmt::{fmt as log_fmt, MakeWriter};

    use super::*;

    /// In-memory sink for the lines one call logs. The `Arc` is shared by every
    /// writer the subscriber makes and by the test that reads it back.
    #[derive(Clone, Default)]
    struct LogSink(Arc<Mutex<Vec<u8>>>);

    impl Write for LogSink {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            self.0.lock().expect("sink lock").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for LogSink {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// What `build` logged, and what it returned.
    fn logged<T>(build: impl FnOnce() -> T) -> (T, String) {
        let sink = LogSink::default();
        let subscriber = log_fmt()
            .with_writer(sink.clone())
            .with_ansi(false)
            .finish();
        let returned = with_default(subscriber, build);
        let bytes = sink.0.lock().expect("sink lock");
        (returned, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// carnet#1407: legal, and never quiet.
    #[test]
    fn a_zero_destructive_budget_warns_from_the_row() {
        let line = "max_destructive_per_turn is 0";
        let zero = GuardianConfigDocument {
            max_destructive_per_turn: Some(0),
            ..GuardianConfigDocument::default()
        };

        let (_, from_row) = logged(|| GuardianConfigRegistry::with_document(zero));
        assert!(
            from_row.contains(" WARN ") && from_row.contains(line),
            "{from_row}"
        );

        let (_, defaults) = logged(GuardianConfigRegistry::bootstrap);
        assert!(!defaults.contains(line), "{defaults}");
    }

    /// carnet#1471: a row written before `mode` and `schema_version` were
    /// retired still loads, with its caps in force and nothing degraded.
    #[test]
    fn a_row_in_the_retired_shape_loads_its_caps_quietly() {
        let row = r#"{"schema_version":2,"mode":"observe","max_destructive_per_turn":1,"max_writes_per_turn":7}"#;
        let (registry, lines) = logged(|| GuardianConfigRegistry::from_persisted_value(row));
        let snapshot = registry.current();

        assert_eq!(
            snapshot.degraded_reason(),
            None,
            "bug: a retired key degraded a row this binary must read"
        );
        let policy = snapshot.guardian.policy();
        assert_eq!(
            policy.max_destructive_per_turn, 1,
            "bug: the stored destructive cap is not in force"
        );
        assert_eq!(
            policy.max_writes_per_turn, 7,
            "bug: the stored write cap is not in force"
        );
        assert!(
            !lines.contains(" ERROR "),
            "bug: a retired key logged an error: {lines}"
        );
        assert!(
            lines.contains("retired fields"),
            "bug: the retired keys were ignored silently: {lines}"
        );

        let persisted = parse_persisted_document(row).expect("today's row parses");
        assert_eq!(
            persisted.retired_fields,
            ["mode", "schema_version"],
            "bug: the retired keys were not recognised"
        );
        assert_eq!(
            persisted.unknown_fields,
            Vec::<String>::new(),
            "bug: a retired key was reported as unknown, and a save would warn it dropped it"
        );
    }

    /// The poll's guard against reverting a save: a tick that read the row just
    /// before a PUT installed on this instance loses the swap.
    #[test]
    fn a_tick_holding_a_snapshot_a_save_replaced_cannot_put_the_old_row_back() {
        let registry = GuardianConfigRegistry::bootstrap();
        let held_by_tick = registry.current();

        let saved = GuardianConfigDocument {
            max_destructive_per_turn: Some(1),
            ..GuardianConfigDocument::default()
        };
        registry.install(saved.clone());

        let row_the_tick_read = GuardianConfigDocument {
            max_writes_per_turn: Some(9),
            ..GuardianConfigDocument::default()
        };
        assert_eq!(
            registry.settle(&held_by_tick, row_the_tick_read),
            GuardianReloadOutcome::Unchanged
        );
        assert_eq!(*registry.current().document, saved);
    }
}
