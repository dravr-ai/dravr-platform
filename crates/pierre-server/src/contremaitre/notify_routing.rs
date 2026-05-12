// ABOUTME: Contremaitre-backed RoutingProvider for dravr-tronc NotifyLayer
// ABOUTME: Hot-reloadable per-event Slack rules parsed from config/notify-routing.yaml
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Contremaitre-backed [`RoutingProvider`] for the dravr-tronc
//! [`NotifyLayer`](dravr_tronc::notify::NotifyLayer).
//!
//! Source-of-truth file: `config/notify-routing.yaml` in dravr-contremaitre,
//! registered under `manifest.json::config.notify-routing`. The same
//! contremaitre sync ticker that hot-reloads cageux and persona contracts
//! re-parses this YAML and installs a fresh routing table — muting a noisy
//! event or rerouting it to a different Slack channel is a YAML edit and a
//! ~60 s wait, no Cloud Run redeploy. See ADR-014 for the full design.
//!
//! ## Layered design
//!
//! - **YAML** lives in contremaitre — defaults + sparse per-event overrides.
//! - **[`RoutingTable`]** — parsed snapshot: one `defaults` rule plus a
//!   map of partial overrides keyed by event name.
//! - **[`ContremaitreRoutingProvider`]** — thread-safe registry holding an
//!   `Arc<RoutingTable>` behind an `RwLock`. The `RoutingProvider` impl
//!   merges overrides on top of defaults to produce the [`RoutingRule`] the
//!   layer applies. Boot-time state is `defaults: enabled=false` so the
//!   layer drops every event until the first sync installs real rules,
//!   matching the platform-wide "no Slack noise before the prompts are
//!   loaded" invariant.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use dravr_tronc::notify::{BatchRule, DedupRule, RoutingProvider, RoutingRule};
use serde::Deserialize;

use crate::errors::{AppError, AppResult};

/// Process-wide [`ContremaitreRoutingProvider`] shared by the tracing
/// [`NotifyLayer`](dravr_tronc::notify::NotifyLayer) (installed at
/// logging-init time) and the contremaitre sync engine (which calls
/// [`ContremaitreRoutingProvider::reload`] on every tick).
///
/// The layer is wired into the subscriber before `ServerContext::new`
/// builds the rest of the registries, so a `ServerResources` field cannot
/// be the source of truth — by the time sync runs, the layer is already
/// holding an `Arc<R>` and can't be re-pointed. A `LazyLock<Arc<…>>`
/// instead lets every consumer clone the same `Arc` lazily on first use
/// without explicit boot-order plumbing.
pub static NOTIFY_ROUTING_PROVIDER: LazyLock<Arc<ContremaitreRoutingProvider>> =
    LazyLock::new(|| Arc::new(ContremaitreRoutingProvider::new_empty()));

/// Parsed snapshot of `config/notify-routing.yaml`. Read-only once built.
///
/// [`ContremaitreRoutingProvider`] swaps a fresh `Arc<RoutingTable>` in on
/// every successful sync; in-flight `route_for` lookups continue to read
/// whichever snapshot they captured at the start of the call. The
/// `overlay_sha256` is the SHA-256 of the YAML document that produced the
/// table — the contremaitre sync uses it to short-circuit when the
/// manifest entry hash already matches the installed snapshot.
#[derive(Debug)]
struct RoutingTable {
    defaults: RoutingRule,
    overrides: HashMap<String, EventOverride>,
    overlay_sha256: Option<String>,
}

/// Sparse per-event override merged on top of [`RoutingTable::defaults`]
/// inside [`ContremaitreRoutingProvider::route_for`]. Every field is
/// `Option` because the YAML schema is sparse — only keys present in the
/// document touch the merged rule.
#[derive(Debug, Default, Clone)]
struct EventOverride {
    channel: Option<String>,
    enabled: Option<bool>,
    sample_rate: Option<f32>,
    enabled_envs: Option<Vec<String>>,
    dedup: Option<DedupRule>,
    batch: Option<BatchRule>,
}

impl Default for RoutingTable {
    fn default() -> Self {
        Self {
            // Boot snapshot: defaults disabled so no Slack traffic until
            // contremaitre installs the real rules. Channel string is
            // illustrative — `enabled=false` short-circuits the layer
            // before any Slack call is made.
            defaults: RoutingRule {
                channel: "#pierre-events".to_owned(),
                enabled: false,
                dedup: None,
                batch: None,
                sample_rate: 1.0,
                enabled_envs: None,
            },
            overrides: HashMap::new(),
            overlay_sha256: None,
        }
    }
}

/// Hot-reloadable per-event Slack routing rules served to
/// [`NotifyLayer`](dravr_tronc::notify::NotifyLayer) via the
/// [`RoutingProvider`] trait.
///
/// Built at platform boot with [`Self::new_empty`]; the contremaitre sync
/// (see [`super::sync::sync_notify_routing`]) calls [`Self::reload`] on
/// every tick to swap in the latest YAML. Until the first reload succeeds
/// the registry's `defaults.enabled` is `false`, so the layer drops every
/// emit — consistent with cageux / persona-contracts bootstrap behaviour.
pub struct ContremaitreRoutingProvider {
    table: RwLock<Arc<RoutingTable>>,
}

impl Default for ContremaitreRoutingProvider {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl ContremaitreRoutingProvider {
    /// Build a provider with empty overrides and disabled defaults. Safe to
    /// install in a tracing subscriber before contremaitre has synced — the
    /// `NotifyLayer` will short-circuit on `enabled=false` until
    /// [`Self::reload`] runs.
    #[must_use]
    pub fn new_empty() -> Self {
        Self {
            table: RwLock::new(Arc::new(RoutingTable::default())),
        }
    }

    /// Parse `yaml_text` and atomically swap the routing table.
    ///
    /// The previous snapshot stays live if parsing fails — matches
    /// `CageuxConfigRegistry::apply_overlay` semantics so a bad push to
    /// contremaitre cannot silently zero out the routing decisions.
    ///
    /// # Errors
    ///
    /// Returns an error if the YAML fails to parse, the `version` field is
    /// unsupported, or a per-event override carries an invalid duration /
    /// sample rate.
    pub fn reload(&self, yaml_text: &str) -> AppResult<()> {
        self.reload_inner(yaml_text, None)
    }

    /// Reload variant that stashes a precomputed `overlay_sha256` on the
    /// installed snapshot so the contremaitre sync can short-circuit
    /// subsequent ticks when the manifest hash matches.
    ///
    /// # Errors
    ///
    /// Same as [`Self::reload`].
    pub fn reload_with_sha(&self, yaml_text: &str, sha256: String) -> AppResult<()> {
        self.reload_inner(yaml_text, Some(sha256))
    }

    fn reload_inner(&self, yaml_text: &str, sha256: Option<String>) -> AppResult<()> {
        let document: RoutingDocument = serde_yaml::from_str(yaml_text)
            .map_err(|e| AppError::invalid_input(format!("notify-routing YAML: {e}")))?;

        if document.version != ROUTING_SCHEMA_VERSION {
            return Err(AppError::invalid_input(format!(
                "notify-routing YAML: unsupported version {} (expected {ROUTING_SCHEMA_VERSION})",
                document.version
            )));
        }

        let mut table = build_routing_table(document)?;
        table.overlay_sha256 = sha256;
        let mut guard = self.write();
        *guard = Arc::new(table);
        Ok(())
    }

    /// SHA-256 of the YAML overlay that produced the current snapshot, or
    /// `None` for the bootstrap snapshot. The contremaitre sync uses this
    /// to skip reload when the manifest entry hash matches.
    #[must_use]
    pub fn current_sha256(&self) -> Option<String> {
        self.read().overlay_sha256.clone()
    }

    fn read(&self) -> RwLockReadGuard<'_, Arc<RoutingTable>> {
        self.table.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Arc<RoutingTable>> {
        self.table.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Current snapshot for tests and admin diagnostics. Cloning the `Arc`
    /// is cheap.
    #[must_use]
    fn snapshot(&self) -> Arc<RoutingTable> {
        Arc::clone(&self.read())
    }
}

impl RoutingProvider for ContremaitreRoutingProvider {
    fn route_for(&self, event: &str) -> Option<RoutingRule> {
        let table = self.snapshot();
        let merged = merge_rule(&table.defaults, table.overrides.get(event));
        // Defaults can be globally disabled (bootstrap snapshot) — return
        // `None` so the layer takes its own no-rule path instead of trying
        // to post to a placeholder channel.
        if !merged.enabled {
            return None;
        }
        Some(merged)
    }
}

/// Schema version this loader understands. Bumping the YAML version in
/// dravr-contremaitre without updating this constant means the loader
/// rejects the document and the previous snapshot remains live — same
/// pattern as the prompt-manifest version pin.
const ROUTING_SCHEMA_VERSION: u32 = 1;

/// Top-level YAML document. Field-for-field mirror of
/// `config/notify-routing.yaml`.
#[derive(Debug, Deserialize)]
struct RoutingDocument {
    version: u32,
    #[serde(default)]
    defaults: RoutingDefaultsYaml,
    #[serde(default)]
    events: HashMap<String, RoutingOverrideYaml>,
}

/// Defaults block as it appears on disk. Every field is optional so the
/// YAML can declare just a subset — missing fields fall back to the
/// `RoutingRule::to_channel` defaults.
#[derive(Debug, Default, Deserialize)]
struct RoutingDefaultsYaml {
    channel: Option<String>,
    enabled: Option<bool>,
    sample_rate: Option<f32>,
    enabled_envs: Option<Vec<String>>,
    dedup_per: Option<Vec<String>>,
    dedup_window: Option<String>,
    batch: Option<String>,
}

/// Per-event override block. Sparse — only keys present in the YAML touch
/// the merged rule. `dedup_per` + `dedup_window` together produce a
/// [`DedupRule`]; `batch` produces a [`BatchRule`]. Missing the window or
/// the dedup-keys list is a validation error so the loader catches typos
/// at sync time.
#[derive(Debug, Default, Deserialize)]
struct RoutingOverrideYaml {
    channel: Option<String>,
    enabled: Option<bool>,
    sample_rate: Option<f32>,
    enabled_envs: Option<Vec<String>>,
    dedup_per: Option<Vec<String>>,
    dedup_window: Option<String>,
    batch: Option<String>,
}

/// Translate the parsed YAML document into the in-memory [`RoutingTable`].
fn build_routing_table(document: RoutingDocument) -> AppResult<RoutingTable> {
    let defaults = build_default_rule(document.defaults)?;

    let mut overrides: HashMap<String, EventOverride> =
        HashMap::with_capacity(document.events.len());
    for (event, override_yaml) in document.events {
        let entry = build_event_override(&event, override_yaml)?;
        overrides.insert(event, entry);
    }

    // `overlay_sha256` is stamped by `reload_inner` after this function
    // returns — `build_routing_table` does the pure parsing work, while
    // the SHA-tracking concern lives one layer up.
    Ok(RoutingTable {
        defaults,
        overrides,
        overlay_sha256: None,
    })
}

/// Build the `defaults` rule, falling back to `RoutingRule::to_channel`
/// behaviour for fields the YAML omits.
fn build_default_rule(yaml: RoutingDefaultsYaml) -> AppResult<RoutingRule> {
    let channel = yaml.channel.unwrap_or_else(|| "#pierre-events".to_owned());
    let mut rule = RoutingRule::to_channel(channel);
    if let Some(enabled) = yaml.enabled {
        rule.enabled = enabled;
    }
    if let Some(sample_rate) = yaml.sample_rate {
        rule.sample_rate = validate_sample_rate("defaults", sample_rate)?;
    }
    if let Some(envs) = yaml.enabled_envs {
        rule.enabled_envs = Some(envs);
    }
    if let Some(dedup) = build_dedup_rule("defaults", yaml.dedup_per, yaml.dedup_window.as_deref())?
    {
        rule.dedup = Some(dedup);
    }
    if let Some(batch) = yaml.batch {
        rule.batch = Some(BatchRule {
            interval: parse_duration("defaults.batch", &batch)?,
        });
    }
    Ok(rule)
}

/// Build a sparse [`EventOverride`] from one entry in the `events` map.
/// Field-by-field validation — any duration / sample-rate / dedup mistake
/// is reported with the offending event name so YAML authors find it
/// quickly.
fn build_event_override(event: &str, yaml: RoutingOverrideYaml) -> AppResult<EventOverride> {
    let sample_rate = yaml
        .sample_rate
        .map(|rate| validate_sample_rate(event, rate))
        .transpose()?;
    let dedup = build_dedup_rule(event, yaml.dedup_per, yaml.dedup_window.as_deref())?;
    let batch = yaml
        .batch
        .map(|s| {
            parse_duration(&format!("{event}.batch"), &s).map(|interval| BatchRule { interval })
        })
        .transpose()?;
    Ok(EventOverride {
        channel: yaml.channel,
        enabled: yaml.enabled,
        sample_rate,
        enabled_envs: yaml.enabled_envs,
        dedup,
        batch,
    })
}

/// Combine `dedup_per` + `dedup_window` into a [`DedupRule`]. Returns
/// `None` only when neither field is set. Specifying one without the
/// other is rejected so an author cannot leave a half-configured rule on
/// disk.
fn build_dedup_rule(
    context: &str,
    keys: Option<Vec<String>>,
    window: Option<&str>,
) -> AppResult<Option<DedupRule>> {
    match (keys, window) {
        (None, None) => Ok(None),
        (Some(keys), Some(window)) => {
            if keys.is_empty() {
                return Err(AppError::invalid_input(format!(
                    "notify-routing YAML: {context}.dedup_per must list at least one field"
                )));
            }
            Ok(Some(DedupRule {
                keys,
                window: parse_duration(&format!("{context}.dedup_window"), window)?,
            }))
        }
        (Some(_), None) => Err(AppError::invalid_input(format!(
            "notify-routing YAML: {context}.dedup_per set without dedup_window"
        ))),
        (None, Some(_)) => Err(AppError::invalid_input(format!(
            "notify-routing YAML: {context}.dedup_window set without dedup_per"
        ))),
    }
}

/// Validate that `rate` lies in `[0.0, 1.0]`. Anything outside the range
/// is almost certainly a typo (e.g. `sample_rate: 10` meaning "10%") so
/// rejecting early avoids silently dropping every event.
fn validate_sample_rate(context: &str, rate: f32) -> AppResult<f32> {
    if !(0.0..=1.0).contains(&rate) || !rate.is_finite() {
        return Err(AppError::invalid_input(format!(
            "notify-routing YAML: {context}.sample_rate must be in [0.0, 1.0], got {rate}"
        )));
    }
    Ok(rate)
}

/// Parse a humantime-style duration with `s`, `m`, `h`, or `d` suffix
/// (e.g. `5m`, `1h`, `30s`, `2d`). Bare integers are rejected so a
/// runaway "60" can't masquerade as either 60 s or 60 min.
fn parse_duration(context: &str, text: &str) -> AppResult<Duration> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::invalid_input(format!(
            "notify-routing YAML: {context} is empty"
        )));
    }

    let (digits, suffix) = text.split_at(text.len() - 1);
    let value: u64 = digits.parse().map_err(|_| {
        AppError::invalid_input(format!(
            "notify-routing YAML: {context}: cannot parse '{text}' (expected <N>s|m|h|d)"
        ))
    })?;
    let scale: u64 = match suffix {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 60 * 60 * 24,
        other => {
            return Err(AppError::invalid_input(format!(
                "notify-routing YAML: {context}: unknown duration suffix '{other}' (expected s|m|h|d)"
            )));
        }
    };
    let secs = value.checked_mul(scale).ok_or_else(|| {
        AppError::invalid_input(format!(
            "notify-routing YAML: {context}: duration '{text}' overflows u64 seconds"
        ))
    })?;
    Ok(Duration::from_secs(secs))
}

/// Build the [`RoutingRule`] the layer applies to one event by overlaying
/// the per-event sparse override on top of the defaults.
fn merge_rule(defaults: &RoutingRule, override_entry: Option<&EventOverride>) -> RoutingRule {
    let mut merged = defaults.clone();
    if let Some(over) = override_entry {
        if let Some(channel) = &over.channel {
            merged.channel = channel.clone();
        }
        if let Some(enabled) = over.enabled {
            merged.enabled = enabled;
        }
        if let Some(sample_rate) = over.sample_rate {
            merged.sample_rate = sample_rate;
        }
        if over.enabled_envs.is_some() {
            merged.enabled_envs = over.enabled_envs.clone();
        }
        if over.dedup.is_some() {
            merged.dedup = over.dedup.clone();
        }
        if over.batch.is_some() {
            merged.batch = over.batch.clone();
        }
    }
    merged
}
