// ABOUTME: The keyset document: every key version of both families, its state, and the transitions between states
// ABOUTME: Pure data and rules, no I/O; the loader reads it, the controller and the CLI move it by compare-and-set

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The keyset document
//!
//! The system secret [`KEYSET_SECRET`] holds this document as JSON. It is the
//! one source of truth for which key versions exist, which state each is in
//! and which one is each family's primary (ADR-017, carnet#1347):
//!
//! ```text
//! staged ─▶ primary ─▶ decrypt_only ─▶ disabled ─▶ destroyed
//!                                        ▲   │        (operator only)
//!                                        └───┘ key enable
//! ```
//!
//! Every write bumps `revision`, which is what every process's loader
//! compares to decide whether to rebuild its keyring.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::shared::keyring::{KeyFamily, FLEET_WINDOW};
use serde::{Deserialize, Serialize};

/// The system secret the keyset document is stored under.
pub const KEYSET_SECRET: &str = "database_keyset";

/// How long a version stays `disabled` before it may be destroyed: room for
/// a partial restore (one table from a backup, an exported row) that brings
/// back ciphertext under it.
pub const DESTROY_AFTER_DISABLED: Duration = Duration::from_hours(14 * 24);

/// How long the Google sign-in transaction cookie stays valid
/// (`google_oidc::GOOGLE_SIGN_IN_TTL`): a cookie sealed under a demoted key
/// must expire before the key is disabled.
pub const SIGN_IN_COOKIE_TTL: Duration = Duration::from_mins(10);

/// One key version's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyState {
    /// Loaded and looked up, never used for new writes; promoted after
    /// [`FLEET_WINDOW`].
    Staged,
    /// Seals new ciphertext, or writes new hashes.
    Primary,
    /// Loaded to open older ciphertext or find older hashes, while the
    /// controller drains it.
    DecryptOnly,
    /// Not loaded. Reversible with `key enable`.
    Disabled,
    /// Its wrapped key is deleted. Final.
    Destroyed,
}

impl KeyState {
    /// The name the CLI prints.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Primary => "primary",
            Self::DecryptOnly => "decrypt_only",
            Self::Disabled => "disabled",
            Self::Destroyed => "destroyed",
        }
    }

    /// Whether a process loads a key in this state.
    #[must_use]
    pub const fn is_loaded(self) -> bool {
        matches!(self, Self::Staged | Self::Primary | Self::DecryptOnly)
    }
}

/// One key version and when it moved through its states.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyVersion {
    /// Where it is.
    pub state: KeyState,
    /// When it was staged (or, for a key that predates the keyset, adopted).
    pub created_at: DateTime<Utc>,
    /// When it became primary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_at: Option<DateTime<Utc>>,
    /// When it stopped being primary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demoted_at: Option<DateTime<Utc>>,
    /// When the controller first counted it drained; it is disabled on the
    /// next tick that counts zero again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drained_at: Option<DateTime<Utc>>,
    /// When it was disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_at: Option<DateTime<Utc>>,
    /// When it was destroyed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destroyed_at: Option<DateTime<Utc>>,
}

impl KeyVersion {
    fn new(state: KeyState, now: DateTime<Utc>) -> Self {
        Self {
            state,
            created_at: now,
            promoted_at: (state == KeyState::Primary).then_some(now),
            demoted_at: (state == KeyState::DecryptOnly).then_some(now),
            drained_at: None,
            disabled_at: None,
            destroyed_at: None,
        }
    }
}

/// One state change, for the lifecycle record and the `key_lifecycle.*` log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    /// The family.
    pub family: KeyFamily,
    /// The version.
    pub version: u32,
    /// Its state before; `None` for a version this change created.
    pub from: Option<KeyState>,
    /// Its state after.
    pub to: KeyState,
}

impl Transition {
    /// The lifecycle record's message, e.g. "data v2 promoted".
    #[must_use]
    pub fn message(&self) -> String {
        let verb = match self.to {
            KeyState::Staged => "staged",
            KeyState::Primary if self.from.is_none() => "adopted as primary",
            KeyState::Primary => "promoted",
            KeyState::DecryptOnly if self.from.is_none() => "adopted as decrypt_only",
            KeyState::DecryptOnly if self.from == Some(KeyState::Disabled) => "enabled",
            KeyState::DecryptOnly => "demoted",
            KeyState::Disabled => "disabled",
            KeyState::Destroyed => "destroyed",
        };
        format!("{} v{} {verb}", self.family, self.version)
    }

    /// The `key_lifecycle.*` event this change logs under.
    #[must_use]
    pub const fn event(&self) -> &'static str {
        match self.to {
            KeyState::Staged => super::events::STAGED,
            KeyState::Primary => super::events::PROMOTED,
            KeyState::DecryptOnly => super::events::DEMOTED,
            KeyState::Disabled => super::events::DISABLED,
            KeyState::Destroyed => super::events::DESTROYED,
        }
    }

    /// The lifecycle record's metadata.
    #[must_use]
    pub fn metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "family": self.family.as_str(),
            "version": self.version,
            "from": self.from.map(KeyState::as_str),
            "to": self.to.as_str(),
        })
    }
}

/// Every key version of both families.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeysetDocument {
    /// Bumped by every write; the loader rebuilds when it changes.
    pub revision: u64,
    /// Each family's versions.
    pub families: BTreeMap<KeyFamily, BTreeMap<u32, KeyVersion>>,
}

/// `L` as a chrono duration.
fn fleet_window() -> chrono::Duration {
    chrono::Duration::from_std(FLEET_WINDOW).unwrap_or(chrono::Duration::MAX)
}

fn span(d: Duration) -> chrono::Duration {
    chrono::Duration::from_std(d).unwrap_or(chrono::Duration::MAX)
}

impl KeysetDocument {
    /// A fresh database's keyset: data v1 and index v1, both primary.
    #[must_use]
    pub fn fresh(now: DateTime<Utc>) -> Self {
        Self {
            revision: 1,
            families: KeyFamily::ALL
                .into_iter()
                .map(|family| {
                    (
                        family,
                        BTreeMap::from([(1, KeyVersion::new(KeyState::Primary, now))]),
                    )
                })
                .collect(),
        }
    }

    /// The keyset of a legacy layout at `active_version`: that version is
    /// the data primary, every older one `decrypt_only`, and index v1 (the
    /// v1 data key's bytes) the index primary. The transitions it records.
    #[must_use]
    pub fn adopted(active_version: u32, now: DateTime<Utc>) -> (Self, Vec<Transition>) {
        let active = active_version.max(1);
        let data: BTreeMap<u32, KeyVersion> = (1..=active)
            .map(|version| {
                let state = if version == active {
                    KeyState::Primary
                } else {
                    KeyState::DecryptOnly
                };
                (version, KeyVersion::new(state, now))
            })
            .collect();
        let mut transitions: Vec<Transition> = data
            .iter()
            .map(|(version, key)| Transition {
                family: KeyFamily::Data,
                version: *version,
                from: None,
                to: key.state,
            })
            .collect();
        transitions.push(Transition {
            family: KeyFamily::Index,
            version: 1,
            from: None,
            to: KeyState::Primary,
        });
        let doc = Self {
            revision: 1,
            families: BTreeMap::from([
                (KeyFamily::Data, data),
                (
                    KeyFamily::Index,
                    BTreeMap::from([(1, KeyVersion::new(KeyState::Primary, now))]),
                ),
            ]),
        };
        (doc, transitions)
    }

    /// Parse a stored document.
    ///
    /// # Errors
    ///
    /// When it is not a keyset document, or a family lacks exactly one
    /// primary.
    pub fn parse(text: &str) -> AppResult<Self> {
        let doc: Self = serde_json::from_str(text)
            .map_err(|e| AppError::internal(format!("The stored keyset is not valid: {e}")))?;
        for family in KeyFamily::ALL {
            doc.primary(family)?;
        }
        Ok(doc)
    }

    /// The stored form.
    ///
    /// # Errors
    ///
    /// When serialization fails.
    pub fn to_text(&self) -> AppResult<String> {
        serde_json::to_string(self)
            .map_err(|e| AppError::internal(format!("Failed to serialize the keyset: {e}")))
    }

    /// `family`'s versions.
    #[must_use]
    pub fn versions(&self, family: KeyFamily) -> BTreeMap<u32, KeyVersion> {
        self.families.get(&family).cloned().unwrap_or_default()
    }

    /// `family`'s primary version.
    ///
    /// # Errors
    ///
    /// When the family has no primary, or more than one.
    pub fn primary(&self, family: KeyFamily) -> AppResult<u32> {
        let primaries: Vec<u32> = self
            .versions(family)
            .into_iter()
            .filter(|(_, key)| key.state == KeyState::Primary)
            .map(|(version, _)| version)
            .collect();
        match primaries.as_slice() {
            [one] => Ok(*one),
            _ => Err(AppError::internal(format!(
                "The keyset's {family} family has {} primaries; it must have exactly one",
                primaries.len()
            ))),
        }
    }

    /// `family`'s versions a process loads, ascending.
    #[must_use]
    pub fn loaded(&self, family: KeyFamily) -> Vec<u32> {
        self.versions(family)
            .into_iter()
            .filter(|(_, key)| key.state.is_loaded())
            .map(|(version, _)| version)
            .collect()
    }

    /// The version a new key of `family` takes, refusing while the family
    /// already rotates (a `staged` or `decrypt_only` key): one rotation per
    /// family at a time, so a lookup never needs more than two index keys.
    ///
    /// # Errors
    ///
    /// When the family is mid-rotation.
    pub fn next_version(&self, family: KeyFamily) -> AppResult<u32> {
        let versions = self.versions(family);
        if let Some((version, key)) = versions
            .iter()
            .find(|(_, key)| matches!(key.state, KeyState::Staged | KeyState::DecryptOnly))
        {
            return Err(AppError::invalid_input(format!(
                "{family} v{version} is still {}; one rotation per family at a time",
                key.state.as_str()
            )));
        }
        Ok(versions.keys().next_back().copied().unwrap_or(0) + 1)
    }

    /// Record `version` of `family` as staged.
    ///
    /// # Errors
    ///
    /// When the family is mid-rotation or `version` is not the next one.
    pub fn stage(
        &mut self,
        family: KeyFamily,
        version: u32,
        now: DateTime<Utc>,
    ) -> AppResult<Transition> {
        let next = self.next_version(family)?;
        if version != next {
            return Err(AppError::invalid_input(format!(
                "{family} v{version} is not the next version (v{next})"
            )));
        }
        self.families
            .entry(family)
            .or_default()
            .insert(version, KeyVersion::new(KeyState::Staged, now));
        self.revision += 1;
        Ok(Transition {
            family,
            version,
            from: None,
            to: KeyState::Staged,
        })
    }

    /// Promote every key staged for at least [`FLEET_WINDOW`]; the old
    /// primary becomes `decrypt_only`. The transitions made.
    pub fn promote_due(&mut self, now: DateTime<Utc>) -> Vec<Transition> {
        let mut made = Vec::new();
        for family in KeyFamily::ALL {
            let Some(versions) = self.families.get_mut(&family) else {
                continue;
            };
            let due = versions
                .iter()
                .find(|(_, key)| {
                    key.state == KeyState::Staged && now - key.created_at >= fleet_window()
                })
                .map(|(version, _)| *version);
            let Some(staged) = due else {
                continue;
            };
            for (version, key) in versions.iter_mut() {
                if key.state == KeyState::Primary {
                    key.state = KeyState::DecryptOnly;
                    key.demoted_at = Some(now);
                    made.push(Transition {
                        family,
                        version: *version,
                        from: Some(KeyState::Primary),
                        to: KeyState::DecryptOnly,
                    });
                }
            }
            if let Some(key) = versions.get_mut(&staged) {
                key.state = KeyState::Primary;
                key.promoted_at = Some(now);
                made.push(Transition {
                    family,
                    version: staged,
                    from: Some(KeyState::Staged),
                    to: KeyState::Primary,
                });
            }
        }
        if !made.is_empty() {
            self.revision += 1;
        }
        made
    }

    /// The `decrypt_only` versions whose disable wait has passed: the fleet
    /// window plus the sign-in cookie's lifetime since demotion.
    #[must_use]
    pub fn disable_candidates(&self, now: DateTime<Utc>) -> Vec<(KeyFamily, u32, DateTime<Utc>)> {
        let wait = fleet_window() + span(SIGN_IN_COOKIE_TTL);
        KeyFamily::ALL
            .into_iter()
            .flat_map(|family| {
                self.versions(family)
                    .into_iter()
                    .filter_map(move |(version, key)| {
                        let demoted = key.demoted_at?;
                        (key.state == KeyState::DecryptOnly && now - demoted >= wait)
                            .then_some((family, version, demoted))
                    })
            })
            .collect()
    }

    /// Record a drain count for a `decrypt_only` version: a zero after an
    /// earlier zero disables it; a first zero marks it drained; anything
    /// else clears the mark. The transition, when it was disabled.
    pub fn record_drain(
        &mut self,
        family: KeyFamily,
        version: u32,
        drained: bool,
        now: DateTime<Utc>,
    ) -> Option<Transition> {
        let key = self.families.get_mut(&family)?.get_mut(&version)?;
        if key.state != KeyState::DecryptOnly {
            return None;
        }
        match (drained, key.drained_at) {
            (true, Some(_)) => {
                key.state = KeyState::Disabled;
                key.disabled_at = Some(now);
                key.drained_at = None;
                self.revision += 1;
                Some(Transition {
                    family,
                    version,
                    from: Some(KeyState::DecryptOnly),
                    to: KeyState::Disabled,
                })
            }
            (true, None) => {
                key.drained_at = Some(now);
                self.revision += 1;
                None
            }
            (false, Some(_)) => {
                key.drained_at = None;
                self.revision += 1;
                None
            }
            (false, None) => None,
        }
    }

    /// Move a `disabled` version back to `decrypt_only`.
    ///
    /// # Errors
    ///
    /// When the version is not disabled, or it is an index key while the
    /// index family already holds two loaded keys: a lookup binds at most
    /// two, so a third would fail every process's keyring build.
    pub fn enable(
        &mut self,
        family: KeyFamily,
        version: u32,
        now: DateTime<Utc>,
    ) -> AppResult<Transition> {
        if family == KeyFamily::Index && self.loaded(KeyFamily::Index).len() >= 2 {
            return Err(AppError::invalid_input(format!(
                "index v{version} cannot be enabled while two index keys are loaded; a lookup binds at most two, so let the rotation in progress finish first"
            )));
        }
        let key = self.key_mut(family, version)?;
        if key.state != KeyState::Disabled {
            return Err(AppError::invalid_input(format!(
                "{family} v{version} is {}, not disabled",
                key.state.as_str()
            )));
        }
        key.state = KeyState::DecryptOnly;
        key.disabled_at = None;
        key.drained_at = None;
        key.demoted_at = Some(now);
        self.revision += 1;
        Ok(Transition {
            family,
            version,
            from: Some(KeyState::Disabled),
            to: KeyState::DecryptOnly,
        })
    }

    /// Whether `version` may be destroyed now: disabled for
    /// [`DESTROY_AFTER_DISABLED`], and for v1 (one stored row serves both
    /// families) both families' v1 disabled or destroyed. The row count is
    /// the caller's.
    ///
    /// # Errors
    ///
    /// Why it may not.
    pub fn check_destroyable(
        &self,
        family: KeyFamily,
        version: u32,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let key = self
            .versions(family)
            .remove(&version)
            .ok_or_else(|| AppError::not_found(format!("{family} v{version}")))?;
        let (KeyState::Disabled, Some(disabled_at)) = (key.state, key.disabled_at) else {
            return Err(AppError::invalid_input(format!(
                "{family} v{version} is {}; only a disabled key is destroyed",
                key.state.as_str()
            )));
        };
        if now - disabled_at < span(DESTROY_AFTER_DISABLED) {
            return Err(AppError::invalid_input(format!(
                "{family} v{version} was disabled at {disabled_at}; it is destroyed no sooner than {} days after",
                DESTROY_AFTER_DISABLED.as_secs() / 86_400
            )));
        }
        if version == 1 {
            let other = match family {
                KeyFamily::Data => KeyFamily::Index,
                KeyFamily::Index => KeyFamily::Data,
            };
            let other_state = self.versions(other).get(&1).map(|k| k.state);
            if other_state.is_some_and(KeyState::is_loaded) {
                return Err(AppError::invalid_input(format!(
                    "v1 is one stored key for both families, and {other} v1 is still in use"
                )));
            }
        }
        Ok(())
    }

    /// Mark a version destroyed (the caller deletes its wrapped key after).
    ///
    /// # Errors
    ///
    /// See [`Self::check_destroyable`].
    pub fn destroy(
        &mut self,
        family: KeyFamily,
        version: u32,
        now: DateTime<Utc>,
    ) -> AppResult<Transition> {
        self.check_destroyable(family, version, now)?;
        let key = self.key_mut(family, version)?;
        key.state = KeyState::Destroyed;
        key.destroyed_at = Some(now);
        self.revision += 1;
        Ok(Transition {
            family,
            version,
            from: Some(KeyState::Disabled),
            to: KeyState::Destroyed,
        })
    }

    /// Whether `family`'s primary is older than `days` and the family is
    /// not already rotating: a scheduled rotation is due. `days == 0` never.
    #[must_use]
    pub fn rotation_due(&self, family: KeyFamily, days: u32, now: DateTime<Utc>) -> bool {
        if days == 0 || self.next_version(family).is_err() {
            return false;
        }
        let Ok(primary) = self.primary(family) else {
            return false;
        };
        self.versions(family).get(&primary).is_some_and(|key| {
            let since = key.promoted_at.unwrap_or(key.created_at);
            now - since >= chrono::Duration::days(i64::from(days))
        })
    }

    fn key_mut(&mut self, family: KeyFamily, version: u32) -> AppResult<&mut KeyVersion> {
        self.families
            .get_mut(&family)
            .and_then(|versions| versions.get_mut(&version))
            .ok_or_else(|| AppError::not_found(format!("{family} v{version}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap() + chrono::Duration::minutes(minutes)
    }

    #[test]
    fn a_fresh_keyset_has_one_primary_per_family_and_round_trips() {
        let doc = KeysetDocument::fresh(at(0));
        assert_eq!(
            doc.primary(KeyFamily::Data).unwrap(),
            1,
            "data v1 is primary"
        );
        assert_eq!(
            doc.primary(KeyFamily::Index).unwrap(),
            1,
            "index v1 is primary"
        );
        let back = KeysetDocument::parse(&doc.to_text().unwrap()).unwrap();
        assert_eq!(
            back, doc,
            "the stored form parses back to the same document"
        );
    }

    #[test]
    fn adopting_a_legacy_layout_keeps_older_versions_decrypt_only() {
        let (doc, transitions) = KeysetDocument::adopted(3, at(0));
        assert_eq!(
            doc.primary(KeyFamily::Data).unwrap(),
            3,
            "the legacy active version is the data primary"
        );
        assert_eq!(
            doc.loaded(KeyFamily::Data),
            vec![1, 2, 3],
            "every legacy version stays loaded"
        );
        assert_eq!(
            doc.versions(KeyFamily::Data)[&1].state,
            KeyState::DecryptOnly,
            "an older legacy version is decrypt_only"
        );
        assert_eq!(
            doc.primary(KeyFamily::Index).unwrap(),
            1,
            "index v1 is the index primary"
        );
        assert_eq!(transitions.len(), 4, "three data versions and index v1");
        assert_eq!(
            transitions[0].message(),
            "data v1 adopted as decrypt_only",
            "the record names an adopted older version"
        );
        assert_eq!(
            transitions[2].message(),
            "data v3 adopted as primary",
            "the record names the adopted primary"
        );
    }

    #[test]
    fn a_staged_key_is_promoted_only_after_the_fleet_window() {
        let mut doc = KeysetDocument::fresh(at(0));
        doc.stage(KeyFamily::Index, 2, at(0)).unwrap();
        assert_eq!(
            doc.loaded(KeyFamily::Index),
            vec![1, 2],
            "a staged key is loaded"
        );
        assert_eq!(
            doc.promote_due(at(14)),
            Vec::new(),
            "nothing is promoted inside the fleet window"
        );
        let made = doc.promote_due(at(15));
        assert_eq!(made.len(), 2, "a demotion and a promotion");
        assert_eq!(
            doc.primary(KeyFamily::Index).unwrap(),
            2,
            "the staged key is primary"
        );
        assert_eq!(
            doc.versions(KeyFamily::Index)[&1].state,
            KeyState::DecryptOnly,
            "the old primary is decrypt_only"
        );
        assert_eq!(
            doc.versions(KeyFamily::Index)[&1].demoted_at,
            Some(at(15)),
            "the demotion is stamped"
        );
        assert_eq!(
            doc.primary(KeyFamily::Data).unwrap(),
            1,
            "the other family is untouched"
        );
    }

    #[test]
    fn one_rotation_per_family_at_a_time() {
        let mut doc = KeysetDocument::fresh(at(0));
        doc.stage(KeyFamily::Data, 2, at(0)).unwrap();
        assert!(
            doc.stage(KeyFamily::Data, 3, at(1)).is_err(),
            "while staged"
        );
        doc.promote_due(at(20));
        assert!(
            doc.next_version(KeyFamily::Data).is_err(),
            "while v1 drains"
        );
        assert_eq!(
            doc.next_version(KeyFamily::Index).unwrap(),
            2,
            "the other family is free"
        );
        assert!(
            doc.stage(KeyFamily::Index, 5, at(1)).is_err(),
            "only the next version"
        );
    }

    #[test]
    fn a_drained_key_is_disabled_on_the_second_zero_after_the_wait() {
        let mut doc = KeysetDocument::fresh(at(0));
        doc.stage(KeyFamily::Data, 2, at(0)).unwrap();
        doc.promote_due(at(15));
        // L + the cookie TTL = 25 minutes after the demotion at 15.
        assert_eq!(
            doc.disable_candidates(at(39)),
            Vec::new(),
            "no candidate before the disable wait"
        );
        assert_eq!(
            doc.disable_candidates(at(40)),
            vec![(KeyFamily::Data, 1, at(15))],
            "v1 is a candidate once the wait has passed"
        );

        assert_eq!(
            doc.record_drain(KeyFamily::Data, 1, true, at(40)),
            None,
            "a first zero only marks the key drained"
        );
        // A row reappeared: the mark is cleared, and the count starts over.
        assert_eq!(
            doc.record_drain(KeyFamily::Data, 1, false, at(45)),
            None,
            "a non-zero count clears the mark"
        );
        assert_eq!(
            doc.record_drain(KeyFamily::Data, 1, true, at(50)),
            None,
            "the count starts over after a non-zero"
        );
        let disabled = doc.record_drain(KeyFamily::Data, 1, true, at(55)).unwrap();
        assert_eq!(
            disabled.message(),
            "data v1 disabled",
            "a second consecutive zero disables"
        );
        assert_eq!(
            doc.loaded(KeyFamily::Data),
            vec![2],
            "a disabled key is not loaded"
        );
        assert!(
            doc.next_version(KeyFamily::Data).is_ok(),
            "the rotation is over"
        );
    }

    #[test]
    fn a_disabled_key_is_enabled_back_or_destroyed_after_fourteen_days() {
        let mut doc = KeysetDocument::fresh(at(0));
        doc.stage(KeyFamily::Data, 2, at(0)).unwrap();
        doc.promote_due(at(15));
        doc.record_drain(KeyFamily::Data, 1, true, at(40));
        doc.record_drain(KeyFamily::Data, 1, true, at(45));

        assert!(
            doc.check_destroyable(KeyFamily::Data, 1, at(45 + 60 * 24 * 13))
                .is_err(),
            "a key disabled for 13 days is not destroyed"
        );
        // v1 is also index v1, which is still primary.
        assert!(
            doc.check_destroyable(KeyFamily::Data, 1, at(45 + 60 * 24 * 15))
                .is_err(),
            "data v1 is not destroyed while index v1 is in use"
        );

        let enabled = doc.enable(KeyFamily::Data, 1, at(50)).unwrap();
        assert_eq!(
            enabled.message(),
            "data v1 enabled",
            "the record names the enable"
        );
        assert_eq!(
            doc.loaded(KeyFamily::Data),
            vec![1, 2],
            "an enabled key is loaded again"
        );
        assert!(
            doc.enable(KeyFamily::Data, 1, at(51)).is_err(),
            "only a disabled key is enabled"
        );
    }

    #[test]
    fn an_index_key_is_not_enabled_while_two_index_keys_are_loaded() {
        let mut doc = KeysetDocument::fresh(at(0));
        doc.stage(KeyFamily::Index, 2, at(0)).unwrap();
        doc.promote_due(at(15));
        doc.record_drain(KeyFamily::Index, 1, true, at(40));
        doc.record_drain(KeyFamily::Index, 1, true, at(45));
        doc.stage(KeyFamily::Index, 3, at(50)).unwrap();
        assert_eq!(
            doc.loaded(KeyFamily::Index),
            vec![2, 3],
            "the test setup leaves k2 primary and k3 staged"
        );
        assert!(
            doc.enable(KeyFamily::Index, 1, at(51)).is_err(),
            "a third loaded index key is refused"
        );
        assert_eq!(
            doc.versions(KeyFamily::Index)[&1].state,
            KeyState::Disabled,
            "a refused enable leaves the key disabled"
        );
        doc.promote_due(at(65));
        doc.record_drain(KeyFamily::Index, 2, true, at(90));
        doc.record_drain(KeyFamily::Index, 2, true, at(95));
        assert!(
            doc.enable(KeyFamily::Index, 1, at(96)).is_ok(),
            "with one index key loaded, enabling a second is allowed"
        );
        assert_eq!(
            doc.loaded(KeyFamily::Index),
            vec![1, 3],
            "the enabled key and the primary are loaded"
        );
    }

    #[test]
    fn version_two_is_destroyed_once_disabled_long_enough() {
        let mut doc = KeysetDocument::fresh(at(0));
        doc.stage(KeyFamily::Data, 2, at(0)).unwrap();
        doc.promote_due(at(15));
        doc.record_drain(KeyFamily::Data, 1, true, at(40));
        doc.record_drain(KeyFamily::Data, 1, true, at(45));
        doc.stage(KeyFamily::Data, 3, at(50)).unwrap();
        doc.promote_due(at(65));
        doc.record_drain(KeyFamily::Data, 2, true, at(90));
        doc.record_drain(KeyFamily::Data, 2, true, at(95));
        let later = at(95 + 60 * 24 * 14);
        let destroyed = doc.destroy(KeyFamily::Data, 2, later).unwrap();
        assert_eq!(destroyed.to, KeyState::Destroyed, "v2 is destroyed");
        assert_eq!(
            doc.next_version(KeyFamily::Data).unwrap(),
            4,
            "a destroyed number is never reused"
        );
    }

    #[test]
    fn a_scheduled_rotation_is_due_by_the_primary_age_alone() {
        let doc = KeysetDocument::fresh(at(0));
        assert!(
            !doc.rotation_due(KeyFamily::Data, 365, at(60 * 24 * 364)),
            "a 364-day-old primary is not due"
        );
        assert!(
            doc.rotation_due(KeyFamily::Data, 365, at(60 * 24 * 365)),
            "a 365-day-old primary is due"
        );
        assert!(
            !doc.rotation_due(KeyFamily::Data, 0, at(60 * 24 * 9999)),
            "0 turns it off"
        );
        let mut rotating = doc;
        rotating.stage(KeyFamily::Data, 2, at(0)).unwrap();
        assert!(
            !rotating.rotation_due(KeyFamily::Data, 365, at(60 * 24 * 400)),
            "a family already rotating is never due"
        );
    }

    #[test]
    fn a_document_without_exactly_one_primary_is_refused() {
        let mut doc = KeysetDocument::fresh(at(0));
        if let Some(index) = doc.families.get_mut(&KeyFamily::Index) {
            index.insert(2, KeyVersion::new(KeyState::Primary, at(0)));
        }
        assert!(
            KeysetDocument::parse(&doc.to_text().unwrap()).is_err(),
            "a family with two primaries is refused"
        );
    }
}
