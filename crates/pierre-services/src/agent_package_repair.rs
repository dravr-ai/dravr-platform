// ABOUTME: Boot repair of stored package flavours — a readiness level whose hard-session cap is 0 opens no quality purpose
// ABOUTME: Rewrites the stored YAML a stricter kernel refuses, in place with its comments when the edit verifies

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Package flavours, repaired at boot
//!
//! `agent_artefacts` keeps a package's flavour as the text the kernel
//! validated when it was seeded. The kernel refuses a readiness ladder in
//! which a level whose `max_hard_sessions_per_week` is 0 still opens a
//! quality purpose — the rule p0 always followed, applied to every cap-0
//! level. Every catalogue flavour once opened `tempo` at p1 with a cap of 0,
//! and a coach package that copied one carries the same line. Such a row no
//! longer parses, and the resolver skips it
//! ([`crate::agent_package::AgentPackage::from_rows`]), so the coach's
//! athletes silently lose the house flavour.
//!
//! [`repair_stored_flavours`] runs once at boot, before the server takes
//! traffic. For each stored flavour the kernel refuses, it takes out every
//! quality purpose a closed level opens — p0, and any level whose cap is 0 —
//! which is exactly what the kernel forbids and nothing more, and it writes
//! the text back only when the kernel accepts the result. A flavour refused
//! for any other reason is left as stored and named in the report: the
//! repair never guesses at a rule it does not own. A row that parses is never
//! touched, so a second boot finds nothing to do.
//!
//! The rewrite keeps the author's text where it can. A one-line
//! `purposes: [..]` list — the layout every catalogue flavour uses — is
//! edited on its own line, and the edit is kept only when the result reads
//! back as exactly the document the structural edit produced. Any other
//! layout is re-serialised from the edited document: the same flavour, with
//! its comments dropped.

use std::collections::BTreeMap;

use pierre_core::errors::AppResult;
use pierre_core::models::periodization::{Flavour, ReadinessLevel, WorkoutPurpose};
use pierre_core::models::{sha256_hex, ArtefactKind};
use pierre_database::RepositoryRegistry;
use serde_yaml::Value;
use tracing::{info, warn};

/// The top-level key a flavour's readiness ladder sits under.
const LADDER_KEY: &str = "readiness_substitution";

/// A stored flavour with the quality purposes its closed levels opened taken
/// out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedLadder {
    /// The rewritten text. The kernel accepts it.
    pub content: String,
    /// Per closed level, the quality purposes taken out of it.
    pub removed: BTreeMap<ReadinessLevel, Vec<WorkoutPurpose>>,
    /// `true` when the text was edited in place — comments, layout and key
    /// order as authored — rather than re-serialised.
    pub layout_kept: bool,
}

/// Take every quality purpose out of the readiness levels that allow no hard
/// session — p0, and any level whose `max_hard_sessions_per_week` is 0.
///
/// `None` when the text is not a YAML document, when no closed level opens a
/// quality purpose, or when the kernel still refuses the result — a flavour
/// broken for some other reason is not this repair's to fix.
#[must_use]
pub fn close_quality_at_cap_zero(text: &str) -> Option<ClosedLadder> {
    let mut doc: Value = serde_yaml::from_str(text).ok()?;
    let removed = close_levels(&mut doc)?;
    let in_place = rewrite_flow_lists(text, &removed);
    let edited_as_intended =
        serde_yaml::from_str::<Value>(&in_place).is_ok_and(|read_back| read_back == doc);
    let (content, layout_kept) = if edited_as_intended {
        (in_place, true)
    } else {
        (serde_yaml::to_string(&doc).ok()?, false)
    };
    Flavour::from_yaml(&content).ok()?;
    Some(ClosedLadder {
        content,
        removed,
        layout_kept,
    })
}

/// What one boot's repair did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RepairReport {
    /// Stored flavours read, across every tenant.
    pub flavours_read: usize,
    /// Flavours rewritten, `agent_id/slug`.
    pub repaired: Vec<String>,
    /// Flavours the kernel refuses for a reason this repair does not answer,
    /// `agent_id/slug: the kernel's message`. Left exactly as stored.
    pub unrepaired: Vec<String>,
}

/// Rewrite every stored package flavour a closed ladder level makes the
/// kernel refuse.
///
/// Walks tenant by tenant, so every query carries its tenant. Idempotent: a
/// flavour that parses is never rewritten.
///
/// # Errors
///
/// Returns the repository error when the tenants or a tenant's flavours
/// cannot be read, or a rewrite cannot be written.
pub async fn repair_stored_flavours(repos: &RepositoryRegistry) -> AppResult<RepairReport> {
    let mut report = RepairReport::default();
    for tenant in repos.tenants.get_all().await? {
        let tenant_id = tenant.id.to_string();
        let rows = repos
            .agent_artefacts
            .list_tenant_artefacts(&tenant_id, ArtefactKind::Flavour)
            .await?;
        for row in rows {
            report.flavours_read += 1;
            let Err(refused) = row.parse() else {
                continue;
            };
            let label = format!("{}/{}", row.agent_id, row.slug);
            let Some(closed) = close_quality_at_cap_zero(&row.content) else {
                warn!(
                    flavour = %label,
                    error = %refused,
                    "stored package flavour refused for a reason the boot repair does not answer; left as stored"
                );
                report.unrepaired.push(format!("{label}: {refused}"));
                continue;
            };
            repos
                .agent_artefacts
                .rewrite_agent_artefact(
                    &tenant_id,
                    &row.id,
                    &closed.content,
                    &sha256_hex(&closed.content),
                )
                .await?;
            let removed: Vec<String> = closed
                .removed
                .iter()
                .map(|(level, purposes)| {
                    let names: Vec<&str> = purposes.iter().map(|p| p.as_str()).collect();
                    format!("{level}: {}", names.join(", "))
                })
                .collect();
            info!(
                flavour = %label,
                removed = ?removed,
                layout_kept = closed.layout_kept,
                "stored package flavour rewritten: a level with no hard session opens no quality purpose"
            );
            report.repaired.push(label);
        }
    }
    Ok(report)
}

/// Take the quality purposes out of every closed level of `doc`'s ladder,
/// returning what each level lost; `None` when no level lost anything.
fn close_levels(doc: &mut Value) -> Option<BTreeMap<ReadinessLevel, Vec<WorkoutPurpose>>> {
    let ladder = doc.get_mut(LADDER_KEY)?.as_mapping_mut()?;
    let mut removed = BTreeMap::new();
    for (key, rule) in ladder.iter_mut() {
        let Some(level) = key.as_str().and_then(level_named) else {
            continue;
        };
        let cap_zero = rule
            .get("max_hard_sessions_per_week")
            .and_then(Value::as_u64)
            == Some(0);
        if level != ReadinessLevel::P0 && !cap_zero {
            continue;
        }
        let Some(purposes) = rule.get_mut("purposes").and_then(Value::as_sequence_mut) else {
            continue;
        };
        let mut dropped = Vec::new();
        purposes.retain(|item| match item.as_str().and_then(purpose_named) {
            Some(purpose) if purpose.is_quality() => {
                dropped.push(purpose);
                false
            }
            _ => true,
        });
        if !dropped.is_empty() {
            removed.insert(level, dropped);
        }
    }
    (!removed.is_empty()).then_some(removed)
}

/// `text` with each closed level's one-line `purposes: [..]` list edited, and
/// every other byte as authored.
fn rewrite_flow_lists(
    text: &str,
    removed: &BTreeMap<ReadinessLevel, Vec<WorkoutPurpose>>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ladder = false;
    let mut level: Option<ReadinessLevel> = None;
    for line in text.split_inclusive('\n') {
        let code = line.split('#').next().unwrap_or_default();
        let key = code.trim();
        if !key.is_empty() && !code.starts_with([' ', '\t']) {
            in_ladder = key.strip_suffix(':') == Some(LADDER_KEY);
            level = None;
        } else if in_ladder {
            if let Some(named) = key.strip_suffix(':').and_then(level_named) {
                level = Some(named);
            } else if let Some(dropped) = level.and_then(|l| removed.get(&l)) {
                if key.starts_with("purposes:") {
                    if let Some(edited) = drop_from_flow_list(line, dropped) {
                        out.push_str(&edited);
                        continue;
                    }
                }
            }
        }
        out.push_str(line);
    }
    out
}

/// `line` with `dropped` taken out of its one-line `[..]` list; `None` when
/// the line holds no such list.
fn drop_from_flow_list(line: &str, dropped: &[WorkoutPurpose]) -> Option<String> {
    let open = line.find('[')?;
    let close = open + line.get(open..)?.find(']')?;
    let kept: Vec<&str> = line
        .get(open + 1..close)?
        .split(',')
        .map(str::trim)
        .filter(|item| {
            let name = item.trim_matches(|c| c == '"' || c == '\'');
            !item.is_empty() && !dropped.iter().any(|p| p.as_str() == name)
        })
        .collect();
    Some(format!(
        "{}[{}]{}",
        line.get(..open)?,
        kept.join(", "),
        line.get(close + 1..)?
    ))
}

/// The readiness level spelled `name`, as the ladder's keys spell it.
fn level_named(name: &str) -> Option<ReadinessLevel> {
    ReadinessLevel::ALL
        .iter()
        .copied()
        .find(|level| level.as_str() == name)
}

/// The workout purpose spelled `name`, as a ladder's `purposes` spell it.
fn purpose_named(name: &str) -> Option<WorkoutPurpose> {
    WorkoutPurpose::ALL
        .iter()
        .copied()
        .find(|purpose| purpose.as_str() == name)
}
