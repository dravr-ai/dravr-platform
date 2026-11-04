// ABOUTME: Which provider calendar a plan push, a prescription or a ledger read goes to, and the refusals when none can
// ABOUTME: The athlete's named or connected calendar-write provider, the ledger's own calendar first among several

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::transport::TransportPolicy;
use pierre_providers::backend_resolver;
use pierre_providers::ProviderRegistry;

/// Name of the descriptor capability a push target must declare, as a refusal
/// and its `details` say it.
pub const CALENDAR_WRITE_CAPABILITY: &str = "calendar_write";

/// The provider a plan or a prescription is written to.
///
/// `requested` is the backend the caller named (already resolved from the
/// name the athlete knows), `connected` the backends the athlete holds a live
/// connection to, and `live` the backends the athlete's push ledger holds
/// live entries on. A named provider must declare
/// [`CALENDAR_WRITE`](pierre_providers::spi::ProviderCapabilities::CALENDAR_WRITE),
/// or the athlete must hold a connection to one that does under the same
/// name: a mirror and its API backend share the name the athlete knows
/// (Garmin's usr/pwd connection beside its Garmin API one), and the write
/// goes through the one that writes. Its connection is checked where the
/// credentials are read.
///
/// Without a name, the target is the connected provider that declares it.
/// When several qualify, the one the ledger already holds entries on wins —
/// so the same athlete keeps landing on the same calendar until they name
/// another, even when they connect a second calendar later. Then a provider
/// that serves its data on request goes before a
/// [`PUSH_ONLY`](pierre_providers::spi::ProviderCapabilities::PUSH_ONLY) one:
/// a push-only provider (Garmin) is connected for the data it pushes to
/// Dravr, and its calendar takes writes only once the athlete granted them
/// there, while a calendar provider beside it was connected for its calendar
/// (Phil, 2026-11-04). Then alphabetical order of backend name.
///
/// # Errors
///
/// Returns an invalid-input error naming the providers that declare the
/// capability when the named one does not. Returns `NoProviderConnected`,
/// carrying `missing_capability` and the providers that would satisfy it,
/// when no connection of the athlete declares it.
pub fn resolve_calendar_target(
    registry: &ProviderRegistry,
    connected: &[&str],
    live: &[&str],
    requested: Option<&str>,
) -> AppResult<String> {
    let writable = |backend: &str| {
        registry
            .get_capabilities(backend)
            .is_some_and(|caps| caps.supports_calendar_write())
    };
    if let Some(requested) = requested {
        if writable(requested) {
            return Ok(requested.to_owned());
        }
        let known_as = backend_resolver::user_facing_name(requested);
        if let Some(sibling) = connected.iter().copied().find(|backend| {
            writable(backend) && backend_resolver::user_facing_name(backend) == known_as
        }) {
            return Ok(sibling.to_owned());
        }
        return Err(not_a_calendar(registry, requested));
    }
    let push_only = |backend: &str| {
        registry
            .get_capabilities(backend)
            .is_some_and(|caps| caps.is_push_only())
    };
    let mut candidates: Vec<&str> = connected
        .iter()
        .copied()
        .filter(|backend| writable(backend))
        .collect();
    candidates.sort_unstable_by_key(|backend| (push_only(backend), *backend));
    candidates.dedup();
    candidates
        .iter()
        .find(|backend| live.contains(backend))
        .or_else(|| candidates.first())
        .map_or_else(
            || Err(missing_calendar_capability(registry)),
            |backend| Ok((*backend).to_owned()),
        )
}

/// The refusal of a named provider that writes no calendar, by the name the
/// athlete knows it under. When another backend of the same name does write
/// one (Garmin's API connection beside its usr/pwd one), the refusal names it
/// as the connection to make.
fn not_a_calendar(registry: &ProviderRegistry, requested: &str) -> AppError {
    let display = registry.get_display_name(requested).unwrap_or(requested);
    let known_as = backend_resolver::user_facing_name(requested);
    let sibling = registry
        .calendar_write_providers()
        .into_iter()
        .find(|backend| backend_resolver::user_facing_name(backend) == known_as);
    let message = sibling.map_or_else(
        || {
            format!(
                "{display} does not write a training calendar (capability \
                 {CALENDAR_WRITE_CAPABILITY}). Providers that do: {}",
                calendar_write_names(registry)
            )
        },
        |api| {
            format!(
                "{display}, as the athlete connected it, does not write a training calendar \
                 (capability {CALENDAR_WRITE_CAPABILITY}); connecting {} through its own \
                 authorization ({api}) does",
                registry.get_display_name(api).unwrap_or(api)
            )
        },
    );
    AppError::invalid_input(message)
}

/// The calendar a read of the push ledger looks under.
///
/// The connected target ([`resolve_calendar_target`], `live` as it takes
/// it). Without one, a calendar the ledger still holds entries on (an
/// athlete who disconnected it reads what Dravr left there); else the first
/// calendar provider whose terms let its items travel over every interface,
/// so a block for an athlete who never connected one is empty rather than an
/// error, and no interface holds that empty block back for the name of a
/// calendar the athlete does not use.
///
/// # Errors
///
/// Returns the missing-capability refusal when no registered provider
/// declares
/// [`CALENDAR_WRITE`](pierre_providers::spi::ProviderCapabilities::CALENDAR_WRITE)
/// at all.
pub fn calendar_read_target(
    registry: &ProviderRegistry,
    connected: &[&str],
    live: &[&str],
) -> AppResult<String> {
    resolve_calendar_target(registry, connected, live, None).or_else(|_| {
        let providers = registry.calendar_write_providers();
        let travels_everywhere = |name: &&&str| {
            registry
                .get_descriptor(name)
                .is_some_and(|d| d.transport_policy() == TransportPolicy::AnyTransport)
        };
        providers
            .iter()
            .find(|name| live.contains(name))
            .or_else(|| providers.iter().find(travels_everywhere))
            .or_else(|| providers.first())
            .map(|name| (*name).to_owned())
            .ok_or_else(|| missing_calendar_capability(registry))
    })
}

/// Whether `backend`'s calendar holds week notes as well as sessions
/// ([`CALENDAR_WEEK_NOTES`](pierre_providers::spi::ProviderCapabilities::CALENDAR_WEEK_NOTES)).
#[must_use]
pub fn calendar_holds_week_notes(registry: &ProviderRegistry, backend: &str) -> bool {
    registry
        .get_capabilities(backend)
        .is_some_and(|caps| caps.supports_calendar_week_notes())
}

fn calendar_write_names(registry: &ProviderRegistry) -> String {
    registry.calendar_write_providers().join(", ")
}

fn missing_calendar_capability(registry: &ProviderRegistry) -> AppError {
    let providers = registry.calendar_write_providers();
    let mut err = AppError::new(
        ErrorCode::NoProviderConnected,
        format!(
            "None of the athlete's connected providers writes a training calendar (capability \
             {CALENDAR_WRITE_CAPABILITY}). Connect one of: {}",
            providers.join(", ")
        ),
    );
    err.details = Some(Box::new(serde_json::json!({
        "action": "connect_provider",
        "missing_capability": CALENDAR_WRITE_CAPABILITY,
        "providers": providers,
    })));
    err
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_providers::spi::{
        OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
    };

    /// A calendar-writing provider the reconciler has never heard of.
    struct MockCalendar {
        name: &'static str,
        capabilities: ProviderCapabilities,
    }

    impl ProviderDescriptor for MockCalendar {
        fn name(&self) -> &'static str {
            self.name
        }
        fn display_name(&self) -> &'static str {
            // The names the platform shows for the Garmin pair; a mock's
            // own slug otherwise.
            match self.name {
                "garmin" => "Garmin Connect",
                "sciotte_garmin" => "Garmin",
                other => other,
            }
        }
        fn capabilities(&self) -> ProviderCapabilities {
            self.capabilities
        }
        fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
            None
        }
        fn oauth_params(&self) -> Option<OAuthParams> {
            None
        }
        fn oauth_refresh(&self) -> Option<OAuthRefresh> {
            None
        }
        fn api_base_url(&self) -> &'static str {
            ""
        }
        fn default_scopes(&self) -> &'static [&'static str] {
            &[]
        }
    }

    fn registry() -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        for (name, capabilities) in [
            (
                "mock_calendar_b",
                ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::CALENDAR_WRITE),
            ),
            (
                "mock_calendar_a",
                ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::CALENDAR_WRITE),
            ),
            (
                "mock_read_only",
                ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::PLANNED_WORKOUTS),
            ),
        ] {
            registry.register_descriptor(name, Box::new(MockCalendar { name, capabilities }));
        }
        registry
    }

    #[test]
    fn the_target_is_the_connected_provider_that_writes_a_calendar() {
        let target =
            resolve_calendar_target(&registry(), &["strava", "mock_calendar_b"], &[], None);
        assert_eq!(target.unwrap(), "mock_calendar_b");
    }

    #[test]
    fn several_qualifying_connections_resolve_alphabetically_whatever_the_order() {
        let registry = registry();
        for connected in [
            ["mock_calendar_b", "mock_calendar_a"],
            ["mock_calendar_a", "mock_calendar_b"],
        ] {
            let target = resolve_calendar_target(&registry, &connected, &[], None);
            assert_eq!(target.unwrap(), "mock_calendar_a");
        }
    }

    #[test]
    fn the_calendar_the_ledger_already_writes_wins_over_the_alphabet() {
        // An athlete pushing to one calendar who connects a second one that
        // sorts first keeps landing on the first: their entries live there.
        let target = resolve_calendar_target(
            &registry(),
            &["mock_calendar_a", "mock_calendar_b"],
            &["mock_calendar_b"],
            None,
        );
        assert_eq!(target.unwrap(), "mock_calendar_b");
        let unconnected = resolve_calendar_target(
            &registry(),
            &["mock_calendar_a", "mock_calendar_b"],
            &["mock_read_only"],
            None,
        );
        assert_eq!(
            unconnected.unwrap(),
            "mock_calendar_a",
            "a ledger on no qualifying connection decides nothing"
        );
    }

    /// The registry with a Garmin API backend that writes a calendar beside
    /// its usr/pwd mirror that does not, as the platform registers them.
    fn garmin_pair() -> ProviderRegistry {
        let mut registry = registry();
        for (name, capabilities) in [
            (
                "garmin",
                ProviderCapabilities::ACTIVITIES
                    .union(ProviderCapabilities::CALENDAR_WRITE)
                    .union(ProviderCapabilities::PUSH_ONLY),
            ),
            ("sciotte_garmin", ProviderCapabilities::ACTIVITIES),
        ] {
            registry.register_descriptor(name, Box::new(MockCalendar { name, capabilities }));
        }
        registry
    }

    #[test]
    fn a_push_only_calendar_goes_after_a_calendar_provider_beside_it() {
        let registry = garmin_pair();
        let fresh = resolve_calendar_target(&registry, &["garmin", "mock_calendar_b"], &[], None);
        assert_eq!(
            fresh.unwrap(),
            "mock_calendar_b",
            "an athlete who never pushed lands on the calendar they connected for its calendar"
        );
        let alone = resolve_calendar_target(&registry, &["garmin"], &[], None);
        assert_eq!(alone.unwrap(), "garmin");
        let pushed =
            resolve_calendar_target(&registry, &["garmin", "mock_calendar_b"], &["garmin"], None);
        assert_eq!(
            pushed.unwrap(),
            "garmin",
            "the ledger's calendar still wins over the ranking"
        );
    }

    #[test]
    fn a_named_mirror_writes_through_the_api_connection_of_the_same_name() {
        let target = resolve_calendar_target(
            &garmin_pair(),
            &["sciotte_garmin", "garmin"],
            &[],
            Some("sciotte_garmin"),
        );
        assert_eq!(target.unwrap(), "garmin");
    }

    #[test]
    fn a_named_mirror_alone_is_refused_with_the_connection_that_writes() {
        let err = resolve_calendar_target(
            &garmin_pair(),
            &["sciotte_garmin"],
            &[],
            Some("sciotte_garmin"),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(err.message.contains("(garmin)"), "{}", err.message);
        assert!(
            !err.message.contains("sciotte_garmin"),
            "a backend slug never reaches a reply: {}",
            err.message
        );
    }

    #[test]
    fn a_named_provider_wins_over_the_default_order() {
        let target = resolve_calendar_target(
            &registry(),
            &["mock_calendar_a", "mock_calendar_b"],
            &["mock_calendar_a"],
            Some("mock_calendar_b"),
        );
        assert_eq!(target.unwrap(), "mock_calendar_b");
    }

    #[test]
    fn a_named_provider_without_the_capability_is_refused_by_name() {
        let err =
            resolve_calendar_target(&registry(), &[], &[], Some("mock_read_only")).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(
            err.message.contains(CALENDAR_WRITE_CAPABILITY),
            "{}",
            err.message
        );
        assert!(err.message.contains("mock_calendar_a"), "{}", err.message);
    }

    #[test]
    fn no_calendar_capable_connection_names_the_missing_capability() {
        let err = resolve_calendar_target(&registry(), &["strava", "mock_read_only"], &[], None)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NoProviderConnected);
        assert!(
            err.message.contains(CALENDAR_WRITE_CAPABILITY),
            "{}",
            err.message
        );
        let details = err.details.expect("details");
        assert_eq!(details["missing_capability"], CALENDAR_WRITE_CAPABILITY);
        let providers = details["providers"].as_array().expect("providers");
        assert!(providers.iter().any(|name| name == "mock_calendar_a"));
        assert!(providers.iter().all(|name| name != "mock_read_only"));
    }

    #[test]
    fn a_read_looks_under_a_calendar_that_travels_everywhere_when_none_is_connected() {
        // The registry also holds whichever real calendar providers this
        // build compiles (garmin, intervals_icu and wahoo under
        // --all-features), so the expected one is read from it rather than
        // assumed: the first whose terms admit every transport, else the
        // first (the mocks declare the descriptor default, first-party only).
        let registry = registry();
        let providers = registry.calendar_write_providers();
        let expected = providers
            .iter()
            .find(|name| {
                registry
                    .get_descriptor(name)
                    .is_some_and(|d| d.transport_policy() == TransportPolicy::AnyTransport)
            })
            .or_else(|| providers.first())
            .copied()
            .expect("a calendar provider");
        let target = calendar_read_target(&registry, &["strava"], &[]);
        assert_eq!(target.unwrap(), expected);
    }

    #[test]
    fn a_read_finds_a_disconnected_calendar_the_ledger_still_holds() {
        let target = calendar_read_target(&registry(), &["strava"], &["mock_calendar_b"]);
        assert_eq!(target.unwrap(), "mock_calendar_b");
    }

    #[test]
    fn a_read_prefers_the_connected_target() {
        let target = calendar_read_target(&registry(), &["mock_calendar_b"], &[]);
        assert_eq!(target.unwrap(), "mock_calendar_b");
    }
}
