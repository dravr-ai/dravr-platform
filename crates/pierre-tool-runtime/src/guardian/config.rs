// ABOUTME: Guardian runtime configuration — persisted document, env overlay, hot-reloadable registry
// ABOUTME: system_settings.guardian_config + GUARDIAN_* env resolve into the Arc<Guardian> every dispatch reads

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
//! A registry swap only mutates the local process (same trade-off as the
//! harness config registry). The dev deployment is single-instance; a
//! multi-instance fleet pins posture via the env overrides instead.

use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::factory::Database;
use serde::{Deserialize, Serialize};
use serde_json::from_str;
use tracing::{info, warn};

use super::policy::{
    ExternalSendAllowlist, GuardianEnvOverrides, GuardianMode, GuardianPolicy, PlanMode,
    TaintedDestructive,
};
use super::Guardian;

/// `system_settings` key holding the persisted [`GuardianConfigDocument`].
pub const GUARDIAN_CONFIG_SETTING_KEY: &str = "guardian_config";

/// Current document schema version. Bump when fields are removed or renamed.
pub const GUARDIAN_CONFIG_SCHEMA_VERSION: u32 = 1;

const fn default_schema_version() -> u32 {
    GUARDIAN_CONFIG_SCHEMA_VERSION
}

/// Top-level document persisted under [`GUARDIAN_CONFIG_SETTING_KEY`].
///
/// Every policy field is optional: an unset field follows the compiled-in
/// default, so a document written today does not freeze today's defaults —
/// when a default hardens in a later release, undocumented fields harden with
/// it. Enum values use the same lowercase wire form as the `GUARDIAN_*` env
/// vars (`"enforce"`, `"log"`, …); `external_send` is `"none"`, `"all"`, or an
/// array of tenant UUIDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Override for [`GuardianPolicy::external_send`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_send: Option<ExternalSendAllowlist>,
    /// Override for [`GuardianPolicy::tainted_destructive`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tainted_destructive: Option<TaintedDestructive>,
    /// Override for [`GuardianPolicy::plan_mode`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_mode: Option<PlanMode>,
}

impl Default for GuardianConfigDocument {
    fn default() -> Self {
        Self {
            schema_version: GUARDIAN_CONFIG_SCHEMA_VERSION,
            mode: None,
            max_destructive_per_turn: None,
            max_writes_per_turn: None,
            external_send: None,
            tainted_destructive: None,
            plan_mode: None,
        }
    }
}

/// Reject broken documents before they reach storage.
///
/// The field types already constrain the value space (enums parse or fail,
/// budgets are `u16`), so the load-bearing checks are the schema version and
/// the one semantic foot-gun: `max_writes_per_turn: 0` would block every
/// write-tool dispatch platform-wide, which is an outage switch, not a
/// budget — an operator wanting a kill switch sets `mode: off`'s inverse
/// (`GUARDIAN_MODE` stays available for that posture).
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
    if doc.max_writes_per_turn == Some(0) {
        return Err(AppError::invalid_input(
            "max_writes_per_turn must be >= 1 (0 would deny every write-tool dispatch)",
        ));
    }
    Ok(())
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
    /// Source of [`GuardianPolicy::external_send`].
    pub external_send: GuardianFieldSource,
    /// Source of [`GuardianPolicy::tainted_destructive`].
    pub tainted_destructive: GuardianFieldSource,
    /// Source of [`GuardianPolicy::plan_mode`].
    pub plan_mode: GuardianFieldSource,
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
            ("external_send", self.external_send),
            ("tainted_destructive", self.tainted_destructive),
            ("plan_mode", self.plan_mode),
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
    /// Loaded from `system_settings.guardian_config` at startup.
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
}

/// Thread-safe registry holding the current Guardian config snapshot.
///
/// Reads take a short read-lock and return an `Arc` clone, so they are safe
/// on the hot dispatch path. Writes ([`Self::install`]) happen only on admin
/// PUT — a couple per day at most.
pub struct GuardianConfigRegistry {
    env: GuardianEnvOverrides,
    snapshot: RwLock<GuardianConfigSnapshot>,
}

impl GuardianConfigRegistry {
    /// Build a registry with no persisted document (defaults + env only).
    #[must_use]
    pub fn bootstrap() -> Self {
        Self::with_env_overrides(
            GuardianEnvOverrides::from_env(),
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
            snapshot: RwLock::new(snapshot),
        }
    }

    /// Load the document from `system_settings.guardian_config` at startup.
    ///
    /// On any read or parse failure, falls back to the empty default document
    /// (defaults + env still apply) so the server boots under the compiled-in
    /// posture. Each fallback path emits a `warn!` with the underlying error
    /// so operators can spot a corrupted row.
    pub async fn from_database(db: &Database) -> Self {
        match db.get_system_setting(GUARDIAN_CONFIG_SETTING_KEY).await {
            Ok(Some(row)) => Self::from_persisted_value(&row.value),
            Ok(None) => {
                info!("guardian_config row absent in system_settings; using compiled-in defaults");
                Self::bootstrap()
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "guardian_config DB load failed; falling back to compiled-in defaults"
                );
                Self::bootstrap()
            }
        }
    }

    fn from_persisted_value(json: &str) -> Self {
        match from_str::<GuardianConfigDocument>(json) {
            Ok(doc) => {
                info!(
                    schema_version = doc.schema_version,
                    "guardian_config loaded from system_settings"
                );
                Self::with_env_overrides(
                    GuardianEnvOverrides::from_env(),
                    doc,
                    GuardianConfigSource::Database,
                )
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "guardian_config row failed to deserialize; falling back to compiled-in defaults"
                );
                Self::bootstrap()
            }
        }
    }

    /// Cheap clone of the entire current snapshot (admin GET / diagnostics).
    #[must_use]
    pub fn current(&self) -> GuardianConfigSnapshot {
        self.read().clone()
    }

    /// The effective guard for the current snapshot. One atomic `Arc` clone —
    /// this is the per-dispatch read behind `ToolRuntime::guardian()`.
    #[must_use]
    pub fn current_guardian(&self) -> Arc<Guardian> {
        Arc::clone(&self.read().guardian)
    }

    /// Replace the current snapshot with a fresh document.
    ///
    /// Called by the admin PUT handler after [`validate_document`] has
    /// succeeded and the row has been persisted (write-then-install, so a DB
    /// failure leaves both stores on the previous values). Re-resolves the
    /// effective policy against the captured env overrides.
    pub fn install(&self, document: GuardianConfigDocument, source: GuardianConfigSource) {
        let next = build_snapshot(&self.env, document, source);
        let mut guard = self.write();
        *guard = next;
    }

    fn read(&self) -> RwLockReadGuard<'_, GuardianConfigSnapshot> {
        self.snapshot.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, GuardianConfigSnapshot> {
        self.snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn build_snapshot(
    env: &GuardianEnvOverrides,
    document: GuardianConfigDocument,
    source: GuardianConfigSource,
) -> GuardianConfigSnapshot {
    let (policy, field_sources) = resolve(&document, env);
    GuardianConfigSnapshot {
        document: Arc::new(document),
        guardian: Arc::new(Guardian::new(policy)),
        field_sources,
        source,
        loaded_at: Utc::now(),
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
) -> (GuardianPolicy, GuardianFieldSources) {
    let defaults = GuardianPolicy::default();
    let (mode, mode_src) = pick(defaults.mode, doc.mode.as_ref(), env.mode.as_ref());
    let (max_destructive_per_turn, destructive_src) = pick(
        defaults.max_destructive_per_turn,
        doc.max_destructive_per_turn.as_ref(),
        env.max_destructive_per_turn.as_ref(),
    );
    let (max_writes_per_turn, writes_src) = pick(
        defaults.max_writes_per_turn,
        doc.max_writes_per_turn.as_ref(),
        env.max_writes_per_turn.as_ref(),
    );
    let (external_send, external_src) = pick(
        defaults.external_send,
        doc.external_send.as_ref(),
        env.external_send.as_ref(),
    );
    let (tainted_destructive, tainted_src) = pick(
        defaults.tainted_destructive,
        doc.tainted_destructive.as_ref(),
        env.tainted_destructive.as_ref(),
    );
    let (plan_mode, plan_src) = pick(
        defaults.plan_mode,
        doc.plan_mode.as_ref(),
        env.plan_mode.as_ref(),
    );
    (
        GuardianPolicy {
            mode,
            max_destructive_per_turn,
            max_writes_per_turn,
            external_send,
            tainted_destructive,
            plan_mode,
        },
        GuardianFieldSources {
            mode: mode_src,
            max_destructive_per_turn: destructive_src,
            max_writes_per_turn: writes_src,
            external_send: external_src,
            tainted_destructive: tainted_src,
            plan_mode: plan_src,
        },
    )
}
