// ABOUTME: The one retention policy of provider data, resolved from activity_retention.* runtime config
// ABOUTME: A per-provider override wins over the default; 0 or unset keeps forever; an athlete's export or upload never expires

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider data retention (carnet#1482, D8/D9).
//!
//! One policy, two runtime parameters read system-wide
//! ([`pierre_config::activity_retention`]): a default every provider takes
//! and a per-provider override. Nothing in code declares a retention, and
//! both start unset, so nothing expires until an operator sets one with
//! `pierre-cli config set`.
//!
//! The retention sweep resolves the policy once per pass and hands it to
//! dravr-barrage, which expires each live source's activities by when they
//! were last read, and applies the same days to the provider's wellness.
//! What the athlete brought in themselves never expires: their exports
//! (barrage's archive sources) and the files they uploaded.

use std::collections::BTreeMap;
use std::time::Duration;

use dravr_barrage::{RetentionPolicy, SourceKey};
use pierre_config::activity_retention::{parse_provider_days, DEFAULT_DAYS_KEY, PROVIDER_DAYS_KEY};
use pierre_core::constants::oauth_providers::is_user_provided;
use pierre_core::errors::AppResult;
use pierre_runtime_context::{AdminConfigLookup, ConfigLookupScope};
use serde_json::Value;
use tracing::warn;

/// Seconds in one day of retention.
const DAY_SECS: u64 = 86_400;

/// How long each provider's data is kept after it was last read from the
/// provider, as the operator set it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderRetention {
    /// Days every provider the overrides do not name keeps; `None` keeps
    /// with no expiry.
    default_days: Option<u64>,
    /// Days per provider key, `0` keeping that provider with no expiry.
    provider_days: BTreeMap<String, u64>,
}

impl ProviderRetention {
    /// The policy `default_days` and `provider_days` name, the two values as
    /// the admin configuration holds them: an integer and a JSON object held
    /// as its text.
    ///
    /// A value that does not read as its parameter's shape keeps data rather
    /// than deletes it: it is logged and taken as unset.
    #[must_use]
    pub fn from_values(default_days: Option<&Value>, provider_days: Option<&Value>) -> Self {
        let default_days = match default_days {
            None | Some(Value::Null) => None,
            Some(value) => value.as_u64().or_else(|| {
                warn!(key = DEFAULT_DAYS_KEY, %value, "retention default is not whole days; nothing expires by default");
                None
            }),
        };
        let provider_days = match provider_days {
            None | Some(Value::Null) => BTreeMap::new(),
            Some(Value::String(text)) => parse_provider_days(text).unwrap_or_else(|reason| {
                warn!(key = PROVIDER_DAYS_KEY, %reason, "per-provider retention does not read; no override applies");
                BTreeMap::new()
            }),
            Some(value) => {
                warn!(key = PROVIDER_DAYS_KEY, %value, "per-provider retention is not a JSON object's text; no override applies");
                BTreeMap::new()
            }
        };
        Self {
            default_days: default_days.filter(|days| *days > 0),
            provider_days,
        }
    }

    /// The policy as the system-wide configuration sets it now.
    ///
    /// # Errors
    ///
    /// The configuration read's error: a pass that cannot read the policy
    /// expires nothing.
    pub async fn resolve(config: &dyn AdminConfigLookup) -> AppResult<Self> {
        let default_days = config
            .get_value(DEFAULT_DAYS_KEY, ConfigLookupScope::global())
            .await?;
        let provider_days = config
            .get_value(PROVIDER_DAYS_KEY, ConfigLookupScope::global())
            .await?;
        Ok(Self::from_values(
            default_days.as_ref(),
            provider_days.as_ref(),
        ))
    }

    /// How long `provider`'s data is kept after its last read; `None` keeps
    /// it with no expiry.
    ///
    /// What the athlete brought in themselves — an export, a file they
    /// uploaded — is theirs, not a copy held under a provider's terms:
    /// expiry never reaches it, its activities or its wellness, whatever the
    /// overrides name.
    #[must_use]
    pub fn ttl_for_provider(&self, provider: &str) -> Option<Duration> {
        if is_user_provided(provider) {
            return None;
        }
        let days = self
            .provider_days
            .get(provider)
            .copied()
            .or(self.default_days)?;
        (days > 0).then(|| Duration::from_secs(days.saturating_mul(DAY_SECS)))
    }

    /// Whether anything expires under this policy.
    #[must_use]
    pub fn expires_anything(&self) -> bool {
        self.default_days.is_some() || self.provider_days.values().any(|days| *days > 0)
    }

    /// The provider keys the overrides name, sorted.
    pub fn overridden_providers(&self) -> impl Iterator<Item = &str> {
        self.provider_days.keys().map(String::as_str)
    }
}

impl RetentionPolicy for ProviderRetention {
    fn ttl(&self, source: SourceKey) -> Option<Duration> {
        // An upload is a live source to barrage, so its archive split alone
        // does not tell what the athlete brought in: the provider key does.
        self.ttl_for_provider(source.as_str())
    }
}

#[cfg(test)]
mod tests {
    use dravr_barrage::LiveSource;
    use pierre_core::constants::oauth_providers::{UPLOAD, USER_PROVIDED_PROVIDERS};
    use serde_json::json;

    use super::*;

    const DAY: Duration = Duration::from_secs(DAY_SECS);

    fn policy(default_days: &Value, provider_days: &str) -> ProviderRetention {
        ProviderRetention::from_values(Some(default_days), Some(&json!(provider_days)))
    }

    #[test]
    fn the_start_values_keep_every_provider_forever() {
        let unset = policy(&json!(0), "{}");
        assert!(!unset.expires_anything(), "the start values expire nothing");
        for source in LiveSource::ALL {
            assert_eq!(unset.ttl(SourceKey::Live(source)), None, "{source:?}");
        }
        assert_eq!(
            ProviderRetention::from_values(None, None),
            unset,
            "absent values read as the start values"
        );
    }

    #[test]
    fn an_override_names_its_provider_days() {
        let strava = policy(&json!(0), r#"{"strava":30}"#);
        assert_eq!(
            strava.ttl(SourceKey::Live(LiveSource::Strava)),
            Some(DAY * 30),
            "the override sets strava's ttl"
        );
        assert_eq!(
            strava.ttl(SourceKey::Live(LiveSource::Garmin)),
            None,
            "a provider not named keeps forever"
        );
        assert_eq!(
            strava.ttl_for_provider("sciotte_garmin"),
            None,
            "a provider not named keeps forever"
        );
        assert!(strava.expires_anything(), "an override expires something");
        assert_eq!(
            strava.overridden_providers().collect::<Vec<_>>(),
            ["strava"],
            "the override names strava alone"
        );
    }

    #[test]
    fn a_provider_the_overrides_do_not_name_takes_the_default() {
        let both = policy(&json!(90), r#"{"strava":30,"garmin":0}"#);
        assert_eq!(
            both.ttl_for_provider("strava"),
            Some(DAY * 30),
            "the override wins over the default"
        );
        assert_eq!(
            both.ttl_for_provider("whoop"),
            Some(DAY * 90),
            "a provider not named takes the default"
        );
        assert_eq!(
            both.ttl_for_provider("not_a_provider"),
            Some(DAY * 90),
            "an unknown provider takes the default"
        );
        assert_eq!(
            both.ttl_for_provider("garmin"),
            None,
            "0 keeps that provider forever over the default"
        );
    }

    #[test]
    fn an_athletes_export_never_expires() {
        let everything = policy(&json!(1), r#"{"strava_archive":1}"#);
        for archive in SourceKey::ALL {
            assert_eq!(everything.ttl(archive), None, "{archive}");
            assert_eq!(
                everything.ttl_for_provider(archive.as_str()),
                None,
                "{archive}: its imported wellness never expires either"
            );
        }
    }

    /// `USER_PROVIDED_PROVIDERS` stays a literal list: pierre-core and the
    /// cache's SQL (whose literals `activity_cache::sql`'s tests hold to it)
    /// know nothing of barrage. Barrage carries the split this policy needs
    /// as its archive sources plus the upload live source, so the list is
    /// pinned to exactly that: a key barrage adds as an archive, or one the
    /// list names that barrage does not know, fails here before an export
    /// could expire.
    #[test]
    fn the_user_provided_keys_are_barrages_archives_and_its_upload() {
        let mut barrage: Vec<&str> = SourceKey::ALL
            .into_iter()
            .map(SourceKey::as_str)
            .chain([LiveSource::Upload.as_str()])
            .collect();
        let mut listed = USER_PROVIDED_PROVIDERS.to_vec();
        barrage.sort_unstable();
        listed.sort_unstable();
        assert_eq!(
            listed, barrage,
            "the listed keys are barrage's archives and its upload"
        );
        for key in USER_PROVIDED_PROVIDERS {
            let source = SourceKey::parse(key).unwrap_or_else(|e| panic!("{key}: {e}"));
            assert_eq!(
                source.is_archive(),
                *key != UPLOAD,
                "{key}: every listed key but the upload is an archive to barrage"
            );
        }
        assert_eq!(
            UPLOAD,
            LiveSource::Upload.as_str(),
            "the upload key is barrage's upload source"
        );
    }

    #[test]
    fn an_athletes_upload_never_expires() {
        let everything = policy(&json!(1), r#"{"upload":1}"#);
        assert_eq!(
            everything.ttl(SourceKey::Live(LiveSource::Upload)),
            None,
            "an upload never expires"
        );
        assert_eq!(
            everything.ttl(SourceKey::Live(LiveSource::Strava)),
            Some(DAY),
            "a provider's copy still takes the default"
        );
    }

    #[test]
    fn a_value_that_does_not_read_keeps_data() {
        let garbled =
            ProviderRetention::from_values(Some(&json!("ten")), Some(&json!("strava=30")));
        assert!(
            !garbled.expires_anything(),
            "a garbled value expires nothing"
        );
        let object = ProviderRetention::from_values(None, Some(&json!({"strava": 30})));
        assert!(
            !object.expires_anything(),
            "an object instead of its text expires nothing"
        );
    }
}
