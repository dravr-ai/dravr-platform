// ABOUTME: Announcing an athlete's provider connect: the provider.connected notify event and the connect entry in their log
// ABOUTME: One helper every connect path calls once the connection is stored; a failed log write never fails the connect

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A provider connect, announced once
//!
//! Every path that connects an athlete to a provider (OAuth, a sciotte
//! sign-in, a confirmed delegated link, an intervals.icu key) calls
//! [`announce`] once the connection is stored. It raises the one
//! `provider.connected` notify event and appends a `connect` entry to the
//! athlete's log in the activity store, which their export proves
//! (carnet#1542).
//!
//! An admin copying a session onto another account and the relay's own
//! registration are not an athlete connecting, so they announce nothing.

use std::sync::Arc;

use pierre_core::models::TenantId;
use pierre_providers::backend_resolver::user_facing_name;
use pierre_runtime_context::ProviderConnectLog;
use tracing::{info, warn};
use uuid::Uuid;

/// How the athlete connected, carried on the notify event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectVia {
    /// A provider's OAuth grant.
    OAuth,
    /// A sciotte sign-in on the provider's own pages.
    Scrape,
    /// A link their coach proposed, confirmed by the athlete.
    Delegated,
    /// An API key the athlete pasted (intervals.icu).
    ApiKey,
}

impl ConnectVia {
    /// The notify event's `via` field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OAuth => "oauth",
            Self::Scrape => "scrape",
            Self::Delegated => "delegated",
            Self::ApiKey => "api_key",
        }
    }
}

/// One stored connection.
#[derive(Debug, Clone, Copy)]
pub struct ProviderConnected<'a> {
    /// The athlete.
    pub user_id: Uuid,
    /// The tenant they connected in.
    pub tenant_id: TenantId,
    /// The backend the connection is stored under (`strava`,
    /// `sciotte_garmin`, `intervals_icu`): the event names its user-facing
    /// provider, the log its activity store source.
    pub backend: &'a str,
    /// How they connected.
    pub via: ConnectVia,
}

/// Announce `connected`: its notify event, then its log entry.
///
/// The event is `provider.connected`; the entry is `connect`, written
/// through `log` (`None` in a context without the activity store). The
/// connection is already stored, so a failed entry is a warning, never the
/// connect's failure.
pub async fn announce(log: Option<&Arc<dyn ProviderConnectLog>>, connected: ProviderConnected<'_>) {
    let ProviderConnected {
        user_id,
        tenant_id,
        backend,
        via,
    } = connected;
    // `user_id`/`tenant_id` ride inline because the PostHog sink drops events
    // it cannot attribute; `provider` is the user-facing name, the axis the
    // disconnect event is measured on too.
    info!(
        target: "notify",
        event = "provider.connected",
        provider = %user_facing_name(backend),
        backend = %backend,
        via = via.as_str(),
        delegated = via == ConnectVia::Delegated,
        user_id = %user_id,
        tenant_id = %tenant_id,
        "user connected fitness provider"
    );
    let Some(log) = log else {
        return;
    };
    if let Err(error) = log.log_connect(user_id, &tenant_id, backend).await {
        warn!(
            user_id = %user_id,
            backend,
            error = %error,
            "the provider connect is stored, but its connect entry was not logged"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fmt::Debug;
    use std::sync::{Mutex, PoisonError};

    use dravr_barrage::record_log::entry::EntryKind;
    use dravr_barrage::store::ActivityRecordStore;
    use dravr_barrage::{LiveSource, Scope, SourceKey};
    use pierre_test_support::db::create_test_db;
    use tracing::callsite::rebuild_interest_cache;
    use tracing::field::{Field, Visit};
    use tracing::subscriber::set_default;
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::Registry;

    use super::*;
    use crate::barrage_keys::memory_records;
    use crate::barrage_live_store::live_store;

    /// The fields of every `provider.connected` event logged while installed.
    #[derive(Clone, Default)]
    struct ConnectedEvents(Arc<Mutex<Vec<BTreeMap<String, String>>>>);

    struct FieldText<'a>(&'a mut BTreeMap<String, String>);

    impl Visit for FieldText<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
    }

    impl<S: Subscriber> Layer<S> for ConnectedEvents {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut fields = BTreeMap::new();
            event.record(&mut FieldText(&mut fields));
            if fields.get("event").map(String::as_str) == Some("provider.connected") {
                self.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(fields);
            }
        }
    }

    #[tokio::test]
    async fn a_connect_is_announced_once_and_logged_under_its_source() {
        let db = create_test_db().await.expect("test db"); // Safe: test assertion
        let records: Arc<dyn ActivityRecordStore> = Arc::new(memory_records());
        let live = live_store(Arc::clone(&records), None, db.repositories());
        let log: Arc<dyn ProviderConnectLog> = live;
        let events = ConnectedEvents::default();
        let _guard = set_default(Registry::default().with(events.clone()));
        rebuild_interest_cache();
        let user_id = Uuid::new_v4();
        let tenant_id = TenantId::from_uuid(Uuid::new_v4());

        announce(
            Some(&log),
            ProviderConnected {
                user_id,
                tenant_id,
                backend: "sciotte_garmin",
                via: ConnectVia::Scrape,
            },
        )
        .await;

        let events = events
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let [event] = events.as_slice() else {
            panic!("expected one provider.connected event, got {events:?}"); // Safe: test assertion
        };
        let field = |name: &str| event.get(name).map(String::as_str);
        assert_eq!(
            field("provider"),
            Some("garmin"),
            "bug: the event does not carry the user-facing name"
        );
        assert_eq!(
            field("backend"),
            Some("sciotte_garmin"),
            "bug: the event does not name the stored backend"
        );
        assert_eq!(
            field("via"),
            Some("scrape"),
            "bug: the event does not say how"
        );
        assert_eq!(
            field("user_id"),
            Some(user_id.to_string().as_str()),
            "bug: the event is not attributed to the athlete"
        );
        assert_eq!(
            field("tenant_id"),
            Some(tenant_id.to_string().as_str()),
            "bug: the event is not attributed to the tenant"
        );
        let entries = records
            .log_entries(&Scope::new(tenant_id.as_uuid(), user_id), 0, 10)
            .await
            .expect("the log reads"); // Safe: test assertion
        let [entry] = entries.as_slice() else {
            panic!("expected one log entry, got {entries:?}"); // Safe: test assertion
        };
        assert_eq!(
            entry.kind,
            EntryKind::Connect,
            "bug: the entry is not a connect"
        );
        assert_eq!(
            entry.source,
            SourceKey::Live(LiveSource::SciotteGarmin),
            "bug: the connect was logged under the wrong source"
        );
        assert_eq!(entry.record_key, None, "a connect names no record");
    }

    #[tokio::test]
    async fn a_provider_the_store_keeps_nothing_of_logs_nothing() {
        let db = create_test_db().await.expect("test db"); // Safe: test assertion
        let records: Arc<dyn ActivityRecordStore> = Arc::new(memory_records());
        let log: Arc<dyn ProviderConnectLog> =
            live_store(Arc::clone(&records), None, db.repositories());
        let user_id = Uuid::new_v4();
        let tenant_id = TenantId::from_uuid(Uuid::new_v4());

        announce(
            Some(&log),
            ProviderConnected {
                user_id,
                tenant_id,
                backend: "not_a_barrage_source",
                via: ConnectVia::OAuth,
            },
        )
        .await;

        let entries = records
            .log_entries(&Scope::new(tenant_id.as_uuid(), user_id), 0, 10)
            .await
            .expect("the log reads"); // Safe: test assertion
        assert!(entries.is_empty(), "no source, no entry: {entries:?}");
    }
}
