// ABOUTME: Guardian runtime configuration — persisted document, env overlay, hot-reloadable registry
// ABOUTME: system_settings.guardian_config + GUARDIAN_* env resolve into the Arc<Guardian> every dispatch reads; each instance re-reads the row

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
//! policy without a server restart.
//!
//! Resolution is per field: compiled-in default ← persisted document field
//! (when set) ← `GUARDIAN_*` env override (when set). Env wins so a
//! fleet-wide env pin beats a runtime admin edit; [`GuardianFieldSources`]
//! records which layer won each field so admin surfaces can show an operator
//! exactly why a value is in effect. Env is captured once at registry
//! construction — it cannot change while the process runs.
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

use std::sync::Arc;

use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::factory::Database;
use serde::{Deserialize, Serialize};
use serde_json::{Error as JsonError, Map, Value};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::policy::{GuardianEnvOverrides, GuardianMode, GuardianPolicy};
use super::Guardian;

/// `system_settings` key holding the persisted [`GuardianConfigDocument`].
pub const GUARDIAN_CONFIG_SETTING_KEY: &str = "guardian_config";

/// Current document schema version. Bump when fields are removed or renamed.
pub const GUARDIAN_CONFIG_SCHEMA_VERSION: u32 = 2;

/// The fields [`GuardianConfigDocument`] reads; any other key in a stored row
/// is semantics this binary would drop.
const DOCUMENT_FIELDS: [&str; 4] = [
    "schema_version",
    "mode",
    "max_destructive_per_turn",
    "max_writes_per_turn",
];

const fn default_schema_version() -> u32 {
    GUARDIAN_CONFIG_SCHEMA_VERSION
}

/// Top-level document persisted under [`GUARDIAN_CONFIG_SETTING_KEY`].
///
/// Every policy field is optional: an unset field follows the compiled-in
/// default, so a document written today does not freeze today's defaults —
/// when a default hardens in a later release, undocumented fields harden with
/// it. `mode` uses the same lowercase wire form as `GUARDIAN_MODE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardianConfigDocument {
    /// Document schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Override for [`GuardianPolicy::mode`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<GuardianMode>,
    /// Override for [`GuardianPolicy::max_destructive_per_turn`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_destructive_per_turn: Option<u16>,
    /// Override for [`GuardianPolicy::max_writes_per_turn`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_writes_per_turn: Option<u16>,
}

impl Default for GuardianConfigDocument {
    fn default() -> Self {
        Self {
            schema_version: GUARDIAN_CONFIG_SCHEMA_VERSION,
            mode: None,
            max_destructive_per_turn: None,
            max_writes_per_turn: None,
        }
    }
}

/// Reject broken documents before they reach storage.
///
/// The field types already constrain the value space (enums parse or fail,
/// budgets are `u16`), so the load-bearing checks are the schema version and
/// the one semantic foot-gun: `max_writes_per_turn: 0` would block every
/// write-tool dispatch platform-wide, which is an outage switch, not a
/// budget. This document has no switch that stops all writes: `mode` and
/// `GUARDIAN_MODE` decide only whether a denial blocks.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when any invariant fails.
pub fn validate_document(doc: &GuardianConfigDocument) -> AppResult<()> {
    if doc.schema_version != GUARDIAN_CONFIG_SCHEMA_VERSION {
        return Err(AppError::invalid_input(format!(
            "unsupported guardian_config schema_version {} (expected {})",
            doc.schema_version, GUARDIAN_CONFIG_SCHEMA_VERSION
        )));
    }
    (doc.max_writes_per_turn != Some(0)).ok_or_else(|| {
        AppError::invalid_input(
            "max_writes_per_turn must be >= 1 (0 would deny every write-tool dispatch)",
        )
    })
}

/// A stored `guardian_config` row, read as this binary understands it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedGuardianDocument {
    /// The fields this binary reads, stamped with the current schema version,
    /// so a client echoing it back can save.
    pub document: GuardianConfigDocument,
    /// The version the row carried, when it is not the current one.
    pub stored_schema_version: Option<u32>,
    /// Keys in the row this binary does not read.
    pub unknown_fields: Vec<String>,
}

/// Parse a stored row without silently losing what it holds.
///
/// Only a JSON object is a document: serde would otherwise read `[]` as a
/// struct of defaults and call a corrupt row valid.
///
/// # Errors
///
/// Returns the parse error when the row is not an object of valid fields.
pub fn parse_persisted_document(raw: &str) -> Result<PersistedGuardianDocument, JsonError> {
    let fields: Map<String, Value> = serde_json::from_str(raw)?;
    let unknown_fields = fields
        .keys()
        .filter(|key| !DOCUMENT_FIELDS.contains(&key.as_str()))
        .cloned()
        .collect();
    let mut document: GuardianConfigDocument = serde_json::from_value(Value::Object(fields))?;
    let stored_schema_version = (document.schema_version != GUARDIAN_CONFIG_SCHEMA_VERSION)
        .then_some(document.schema_version);
    document.schema_version = GUARDIAN_CONFIG_SCHEMA_VERSION;
    Ok(PersistedGuardianDocument {
        document,
        stored_schema_version,
        unknown_fields,
    })
}

/// Which resolution layer supplied a policy field's effective value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardianFieldSource {
    /// Compiled-in default — neither the document nor env set the field.
    Default,
    /// The persisted `system_settings.guardian_config` document.
    Database,
    /// A `GUARDIAN_*` env var (pins the field; admin edits are shadowed).
    Env,
}

/// Per-field [`GuardianFieldSource`] map for the effective policy.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct GuardianFieldSources {
    /// Source of [`GuardianPolicy::mode`].
    pub mode: GuardianFieldSource,
    /// Source of [`GuardianPolicy::max_destructive_per_turn`].
    pub max_destructive_per_turn: GuardianFieldSource,
    /// Source of [`GuardianPolicy::max_writes_per_turn`].
    pub max_writes_per_turn: GuardianFieldSource,
}

impl GuardianFieldSources {
    /// Names of the fields pinned by env vars (shadowing any admin edit).
    #[must_use]
    pub fn env_pinned(&self) -> Vec<&'static str> {
        let mut pinned = Vec::new();
        for (name, source) in [
            ("mode", self.mode),
            ("max_destructive_per_turn", self.max_destructive_per_turn),
            ("max_writes_per_turn", self.max_writes_per_turn),
        ] {
            if source == GuardianFieldSource::Env {
                pinned.push(name);
            }
        }
        pinned
    }
}

/// Where the current snapshot's document came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardianConfigSource {
    /// No persisted row — the document is the empty default.
    Defaults,
    /// Read from `system_settings.guardian_config`, at startup or on a poll.
    Database,
    /// Installed by the admin PUT handler after a successful write.
    AdminUpdate,
}

/// Snapshot held by the registry: the source document plus the pre-resolved
/// [`Guardian`] so a per-dispatch read is one `Arc` clone.
#[derive(Debug, Clone)]
pub struct GuardianConfigSnapshot {
    /// Document as persisted (env overrides NOT folded in).
    pub document: Arc<GuardianConfigDocument>,
    /// Effective guard: defaults ← document ← env, pre-built per snapshot.
    pub guardian: Arc<Guardian>,
    /// Which layer won each field.
    pub field_sources: GuardianFieldSources,
    /// Where the document came from.
    pub source: GuardianConfigSource,
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
    env: GuardianEnvOverrides,
    snapshot: ArcSwap<GuardianConfigSnapshot>,
    instance_nonce: Uuid,
}

impl GuardianConfigRegistry {
    /// Build a registry with no persisted document and no operator
    /// overrides: the compiled-in defaults.
    #[must_use]
    pub fn bootstrap() -> Self {
        Self::defaults_with(GuardianEnvOverrides::default())
    }

    /// A registry with no persisted document: the compiled-in defaults under
    /// the operator's `env` overrides.
    fn defaults_with(env: GuardianEnvOverrides) -> Self {
        Self::with_env_overrides(
            env,
            GuardianConfigDocument::default(),
            GuardianConfigSource::Defaults,
        )
    }

    /// Build a registry from explicit env overrides and a document.
    ///
    /// The env-free constructor tests use to exercise resolution without
    /// touching process-global env vars.
    #[must_use]
    pub fn with_env_overrides(
        env: GuardianEnvOverrides,
        document: GuardianConfigDocument,
        source: GuardianConfigSource,
    ) -> Self {
        let snapshot = build_snapshot(&env, document, source);
        Self {
            env,
            snapshot: ArcSwap::from_pointee(snapshot),
            instance_nonce: Uuid::new_v4(),
        }
    }

    /// A registry on the defaults because the row could not be used at boot.
    fn degraded_defaults(env: GuardianEnvOverrides, reason: String) -> Self {
        let registry = Self::defaults_with(env);
        registry.snapshot.store(Arc::new(GuardianConfigSnapshot {
            load_failure: Some(reason),
            ..(*registry.current()).clone()
        }));
        registry
    }

    /// Load the document from `system_settings.guardian_config` at startup,
    /// under the operator's `env` overrides (the server's
    /// `ServerConfig.guardian`, parsed).
    ///
    /// On any read or parse failure, falls back to the empty default document
    /// (defaults + env still apply) so the server boots under the compiled-in
    /// posture, marks the snapshot degraded so the admin GET says so, and logs
    /// at `error!`. The first poll that reads a good row repairs it.
    pub async fn from_database(db: &Database, env: GuardianEnvOverrides) -> Self {
        match db.get_system_setting(GUARDIAN_CONFIG_SETTING_KEY).await {
            Ok(Some(row)) => Self::from_persisted_value(&row.value, env),
            Ok(None) => {
                info!("guardian_config row absent in system_settings; using compiled-in defaults");
                Self::defaults_with(env)
            }
            Err(e) => {
                error!(
                    error = %e,
                    "guardian_config DB load failed; falling back to compiled-in defaults"
                );
                Self::degraded_defaults(
                    env,
                    "the row could not be read at boot; running the compiled-in defaults".into(),
                )
            }
        }
    }

    fn from_persisted_value(json: &str, env: GuardianEnvOverrides) -> Self {
        match parse_persisted_document(json) {
            Ok(persisted) => {
                log_stored_semantics(&persisted);
                Self::with_env_overrides(env, persisted.document, GuardianConfigSource::Database)
            }
            Err(e) => {
                log_invalid_row(&e);
                Self::degraded_defaults(
                    env,
                    format!("the row does not parse ({e}); running the compiled-in defaults"),
                )
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
            return self.settle(
                &before,
                GuardianConfigDocument::default(),
                GuardianConfigSource::Defaults,
            );
        };
        match parse_persisted_document(&row.value) {
            Ok(persisted) => {
                if before.load_failure.is_some() || *before.document != persisted.document {
                    log_stored_semantics(&persisted);
                }
                self.settle(&before, persisted.document, GuardianConfigSource::Database)
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
        source: GuardianConfigSource,
    ) -> GuardianReloadOutcome {
        if before.load_failure.is_none() && *before.document == document {
            return GuardianReloadOutcome::Unchanged;
        }
        let next = Arc::new(build_snapshot(&self.env, document, source));
        let prior = self.snapshot.compare_and_swap(before, Arc::clone(&next));
        if !Arc::ptr_eq(&*prior, before) {
            return GuardianReloadOutcome::Unchanged;
        }
        let policy = next.guardian.policy();
        info!(
            mode = policy.mode.as_str(),
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
    /// failure leaves both stores on the previous values). Re-resolves the
    /// effective policy against the captured env overrides.
    pub fn install(&self, document: GuardianConfigDocument, source: GuardianConfigSource) {
        self.snapshot
            .store(Arc::new(build_snapshot(&self.env, document, source)));
    }
}

fn build_snapshot(
    env: &GuardianEnvOverrides,
    document: GuardianConfigDocument,
    source: GuardianConfigSource,
) -> GuardianConfigSnapshot {
    let (policy, field_sources, refused_fields) = resolve(&document, env);
    GuardianConfigSnapshot {
        document: Arc::new(document),
        guardian: Arc::new(Guardian::new(policy)),
        field_sources,
        source,
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
    let unknown = &persisted.unknown_fields;
    if let Some(stored) = persisted.stored_schema_version {
        error!(
            stored,
            current = GUARDIAN_CONFIG_SCHEMA_VERSION,
            ?unknown,
            "guardian_config row carries a foreign schema_version; applying the fields this binary reads"
        );
    } else if !unknown.is_empty() {
        error!(
            ?unknown,
            "guardian_config row carries fields this binary does not read"
        );
    }
    info!(
        schema_version = GUARDIAN_CONFIG_SCHEMA_VERSION,
        "guardian_config loaded from system_settings"
    );
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
/// never refused and never quiet: logged whichever layer set it, each time a
/// snapshot is built, which is at boot, on a save and on a poll that found a change.
fn warn_on_zero_destructive_budget(budget: u16, source: GuardianFieldSource) {
    if budget == 0 {
        warn!(
            ?source,
            "guardian max_destructive_per_turn is 0: every irreversible tool is denied on every \
             turn while the Guardian enforces"
        );
    }
}

/// Resolve one field: compiled-in default ← document ← env (env wins).
fn pick<T: Clone>(default: T, db: Option<&T>, env: Option<&T>) -> (T, GuardianFieldSource) {
    env.map_or_else(
        || {
            db.map_or_else(
                || (default.clone(), GuardianFieldSource::Default),
                |v| (v.clone(), GuardianFieldSource::Database),
            )
        },
        |v| (v.clone(), GuardianFieldSource::Env),
    )
}

fn resolve(
    doc: &GuardianConfigDocument,
    env: &GuardianEnvOverrides,
) -> (GuardianPolicy, GuardianFieldSources, Vec<String>) {
    let defaults = GuardianPolicy::default();
    let mut refused = Vec::new();
    let doc_writes = usable_write_budget(doc, &mut refused);
    let (mode, mode_src) = pick(defaults.mode, doc.mode.as_ref(), env.mode.as_ref());
    let (max_destructive_per_turn, destructive_src) = pick(
        defaults.max_destructive_per_turn,
        doc.max_destructive_per_turn.as_ref(),
        env.max_destructive_per_turn.as_ref(),
    );
    warn_on_zero_destructive_budget(max_destructive_per_turn, destructive_src);
    let (max_writes_per_turn, writes_src) = pick(
        defaults.max_writes_per_turn,
        doc_writes.as_ref(),
        env.max_writes_per_turn.as_ref(),
    );
    (
        GuardianPolicy {
            mode,
            max_destructive_per_turn,
            max_writes_per_turn,
        },
        GuardianFieldSources {
            mode: mode_src,
            max_destructive_per_turn: destructive_src,
            max_writes_per_turn: writes_src,
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

    /// What building a registry from `env` and `document` logged.
    fn logged_building(env: GuardianEnvOverrides, document: GuardianConfigDocument) -> String {
        let sink = LogSink::default();
        let subscriber = log_fmt()
            .with_writer(sink.clone())
            .with_ansi(false)
            .finish();
        with_default(subscriber, || {
            GuardianConfigRegistry::with_env_overrides(
                env,
                document,
                GuardianConfigSource::Database,
            )
        });
        let bytes = sink.0.lock().expect("sink lock");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// carnet#1407: legal, and never quiet, whichever layer set it.
    #[test]
    fn a_zero_destructive_budget_warns_from_the_row_and_from_the_env() {
        let line = "max_destructive_per_turn is 0";
        let zero = GuardianConfigDocument {
            max_destructive_per_turn: Some(0),
            ..GuardianConfigDocument::default()
        };

        let from_row = logged_building(GuardianEnvOverrides::default(), zero.clone());
        assert!(
            from_row.contains(" WARN ") && from_row.contains(line),
            "{from_row}"
        );

        let pinned = GuardianEnvOverrides {
            max_destructive_per_turn: Some(0),
            ..GuardianEnvOverrides::default()
        };
        let from_env = logged_building(pinned, GuardianConfigDocument::default());
        assert!(from_env.contains(line), "{from_env}");

        // A pin above it is what is in force, so there is nothing to say.
        let shadowed = GuardianEnvOverrides {
            max_destructive_per_turn: Some(2),
            ..GuardianEnvOverrides::default()
        };
        let quiet = logged_building(shadowed, zero);
        assert!(!quiet.contains(line), "{quiet}");

        let defaults = logged_building(
            GuardianEnvOverrides::default(),
            GuardianConfigDocument::default(),
        );
        assert!(!defaults.contains(line), "{defaults}");
    }

    /// The poll's guard against reverting a save: a tick that read the row just
    /// before a PUT installed on this instance loses the swap.
    #[test]
    fn a_tick_holding_a_snapshot_a_save_replaced_cannot_put_the_old_row_back() {
        let registry = GuardianConfigRegistry::bootstrap();
        let held_by_tick = registry.current();

        let saved = GuardianConfigDocument {
            mode: Some(GuardianMode::Observe),
            ..GuardianConfigDocument::default()
        };
        registry.install(saved.clone(), GuardianConfigSource::AdminUpdate);

        let row_the_tick_read = GuardianConfigDocument {
            max_writes_per_turn: Some(9),
            ..GuardianConfigDocument::default()
        };
        assert_eq!(
            registry.settle(
                &held_by_tick,
                row_the_tick_read,
                GuardianConfigSource::Database
            ),
            GuardianReloadOutcome::Unchanged
        );
        assert_eq!(*registry.current().document, saved);
    }
}
