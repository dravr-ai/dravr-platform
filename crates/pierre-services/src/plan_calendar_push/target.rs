// ABOUTME: Which provider calendar a plan push, a prescription or a ledger read goes to, and the refusals when none can
// ABOUTME: A named calendar, then the one the athlete chose, then the ledger's; several and none chosen ask the athlete

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::transport::TransportPolicy;
use pierre_providers::backend_resolver;
use pierre_providers::ProviderRegistry;

/// Name of the descriptor capability a push target must declare, as a refusal
/// and its `details` say it.
pub const CALENDAR_WRITE_CAPABILITY: &str = "calendar_write";

/// What a calendar is chosen among, for one athlete.
#[derive(Debug, Clone, Copy, Default)]
pub struct CalendarCandidates<'names> {
    /// The backends the athlete holds a live connection to.
    pub connected: &'names [&'names str],
    /// The backends the athlete's push ledger holds live entries on.
    pub live: &'names [&'names str],
    /// The calendar the athlete chose for Dravr's pushes, as recorded
    /// (carnet#1393).
    pub preferred: Option<&'names str>,
}

/// The calendar a write goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarTarget {
    /// The backend that writes it.
    pub backend: String,
    /// The calendar the athlete chose, when it was passed over: not among the
    /// connections chosen from, or no longer writing a calendar.
    pub passed_over: Option<String>,
}

/// Where a calendar write goes, or that the athlete must be asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarResolution {
    /// The calendar the write goes to.
    Target(CalendarTarget),
    /// Several of the athlete's connections write a calendar and nothing
    /// chooses between them — no provider named, no usable choice of theirs,
    /// no ledger entries on any — so they are asked rather than one being
    /// guessed (Phil, 2026-11-04).
    Choose {
        /// The calendars to offer, in the ranking's order.
        calendars: Vec<String>,
        /// The calendar the athlete chose, when it was passed over.
        passed_over: Option<String>,
    },
}

/// The connected backends that write a training calendar, in the order the
/// athlete is offered them when asked which one Dravr writes to.
///
/// A provider that serves its data on request goes before a
/// [`PUSH_ONLY`](pierre_providers::spi::ProviderCapabilities::PUSH_ONLY) one —
/// a push-only provider (Garmin) is connected for the data it pushes to
/// Dravr, and its calendar takes writes only once the athlete granted them
/// there, while a calendar provider beside it was connected for its calendar
/// (Phil, 2026-11-04) — then alphabetical order of backend name.
#[must_use]
pub fn connected_calendars<'a>(registry: &ProviderRegistry, connected: &[&'a str]) -> Vec<&'a str> {
    let mut ranked: Vec<&'a str> = connected
        .iter()
        .copied()
        .filter(|backend| writes_calendar(registry, backend))
        .collect();
    ranked.sort_unstable_by_key(|backend| {
        (
            registry
                .get_capabilities(backend)
                .is_some_and(|caps| caps.is_push_only()),
            *backend,
        )
    });
    ranked.dedup();
    ranked
}

fn writes_calendar(registry: &ProviderRegistry, backend: &str) -> bool {
    registry
        .get_capabilities(backend)
        .is_some_and(|caps| caps.supports_calendar_write())
}

/// The calendar among `ranked` that the athlete's recorded choice names: the
/// backend itself, or the one that writes under the same name the athlete
/// knows (a choice recorded against Garmin's mirror lands on its API
/// connection).
fn chosen_among<'a>(ranked: &[&'a str], preferred: &str) -> Option<&'a str> {
    ranked
        .iter()
        .copied()
        .find(|backend| *backend == preferred)
        .or_else(|| {
            let known_as = backend_resolver::user_facing_name(preferred);
            ranked
                .iter()
                .copied()
                .find(|backend| backend_resolver::user_facing_name(backend) == known_as)
        })
}

/// The calendar the athlete's recorded choice lands on among `connected`,
/// when it does: the one case where the push ledger need not be read.
#[must_use]
pub fn chosen_calendar<'a>(
    registry: &ProviderRegistry,
    connected: &[&'a str],
    preferred: Option<&str>,
) -> Option<&'a str> {
    preferred
        .and_then(|preferred| chosen_among(&connected_calendars(registry, connected), preferred))
}

/// Whether any of `connected` writes a training calendar.
#[must_use]
pub fn writes_a_calendar(registry: &ProviderRegistry, connected: &[&str]) -> bool {
    connected
        .iter()
        .any(|backend| writes_calendar(registry, backend))
}

/// The provider a plan or a prescription is written to.
///
/// `requested` is the backend the caller named (already resolved from the
/// name the athlete knows). A named provider must declare
/// [`CALENDAR_WRITE`](pierre_providers::spi::ProviderCapabilities::CALENDAR_WRITE),
/// or the athlete must hold a connection to one that does under the same
/// name: a mirror and its API backend share the name the athlete knows
/// (Garmin's usr/pwd connection beside its Garmin API one), and the write
/// goes through the one that writes. Its connection is checked where the
/// credentials are read.
///
/// Without a name, among the connected providers that declare it:
///
/// 1. the one the athlete chose (`preferred`), when it is among them; a
///    choice that is not is passed over and reported, never followed;
/// 2. the one the ledger already holds entries on, so the same athlete keeps
///    landing on the same calendar;
/// 3. the only one;
/// 4. of several, none of them: the athlete is asked
///    ([`CalendarResolution::Choose`]).
///
/// # Errors
///
/// Returns an invalid-input error naming the providers that declare the
/// capability when the named one does not. Returns `NoProviderConnected`,
/// carrying `missing_capability` and the providers that would satisfy it,
/// when no connection of the athlete declares it.
pub fn resolve_calendar_target(
    registry: &ProviderRegistry,
    candidates: &CalendarCandidates<'_>,
    requested: Option<&str>,
) -> AppResult<CalendarResolution> {
    if let Some(requested) = requested {
        return named_calendar(registry, candidates.connected, requested).map(|backend| {
            CalendarResolution::Target(CalendarTarget {
                backend,
                passed_over: None,
            })
        });
    }
    let ranked = connected_calendars(registry, candidates.connected);
    let chosen = candidates
        .preferred
        .map(|preferred| (preferred, chosen_among(&ranked, preferred)));
    if let Some((_, Some(backend))) = chosen {
        return Ok(CalendarResolution::Target(CalendarTarget {
            backend: backend.to_owned(),
            passed_over: None,
        }));
    }
    let passed_over = chosen.map(|(preferred, _)| preferred.to_owned());
    let target = |backend: &str| {
        Ok(CalendarResolution::Target(CalendarTarget {
            backend: backend.to_owned(),
            passed_over: passed_over.clone(),
        }))
    };
    if let Some(backend) = ranked
        .iter()
        .find(|backend| candidates.live.contains(backend))
    {
        return target(backend);
    }
    match ranked.as_slice() {
        [] => Err(missing_calendar_capability(registry)),
        [only] => target(only),
        several => Ok(CalendarResolution::Choose {
            calendars: several
                .iter()
                .map(|backend| (*backend).to_owned())
                .collect(),
            passed_over,
        }),
    }
}

/// The calendar a named provider resolves to among `connected`: itself when
/// it writes a calendar, else the connection that writes under the same name
/// the athlete knows.
///
/// # Errors
///
/// Returns an invalid-input error, by the name the athlete knows, when
/// neither writes a calendar.
pub fn named_calendar(
    registry: &ProviderRegistry,
    connected: &[&str],
    requested: &str,
) -> AppResult<String> {
    if writes_calendar(registry, requested) {
        return Ok(requested.to_owned());
    }
    let known_as = backend_resolver::user_facing_name(requested);
    connected
        .iter()
        .copied()
        .find(|backend| {
            writes_calendar(registry, backend)
                && backend_resolver::user_facing_name(backend) == known_as
        })
        .map(str::to_owned)
        .ok_or_else(|| not_a_calendar(registry, requested))
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
/// Among the connected calendars, the one the athlete chose, else the one
/// the ledger holds entries on, else the first in the ranking: a read never
/// asks. Without a connected calendar, a calendar the ledger still holds
/// entries on (an athlete who disconnected it reads what Dravr left there);
/// else the first calendar provider whose terms let its items travel over
/// every interface, so a block for an athlete who never connected one is
/// empty rather than an error, and no interface holds that empty block back
/// for the name of a calendar the athlete does not use.
///
/// # Errors
///
/// Returns the missing-capability refusal when no registered provider
/// declares
/// [`CALENDAR_WRITE`](pierre_providers::spi::ProviderCapabilities::CALENDAR_WRITE)
/// at all.
pub fn calendar_read_target(
    registry: &ProviderRegistry,
    candidates: &CalendarCandidates<'_>,
) -> AppResult<String> {
    let ranked = connected_calendars(registry, candidates.connected);
    let connected = candidates
        .preferred
        .and_then(|preferred| chosen_among(&ranked, preferred))
        .or_else(|| {
            ranked
                .iter()
                .copied()
                .find(|backend| candidates.live.contains(backend))
        })
        .or_else(|| ranked.first().copied());
    if let Some(backend) = connected {
        return Ok(backend.to_owned());
    }
    let providers = registry.calendar_write_providers();
    let travels_everywhere = |name: &&&str| {
        registry
            .get_descriptor(name)
            .is_some_and(|d| d.transport_policy() == TransportPolicy::AnyTransport)
    };
    providers
        .iter()
        .find(|name| candidates.live.contains(name))
        .or_else(|| providers.iter().find(travels_everywhere))
        .or_else(|| providers.first())
        .map(|name| (*name).to_owned())
        .ok_or_else(|| missing_calendar_capability(registry))
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

    /// Resolve a write with no recorded choice.
    fn write(
        registry: &ProviderRegistry,
        connected: &[&str],
        live: &[&str],
        requested: Option<&str>,
    ) -> AppResult<CalendarResolution> {
        resolve_calendar_target(
            registry,
            &CalendarCandidates {
                connected,
                live,
                preferred: None,
            },
            requested,
        )
    }

    /// The target of a resolution that chose one.
    fn target(resolution: AppResult<CalendarResolution>) -> CalendarTarget {
        match resolution.expect("resolved") {
            CalendarResolution::Target(target) => target,
            CalendarResolution::Choose { calendars, .. } => {
                panic!("asked the athlete among {calendars:?} instead of choosing")
            }
        }
    }

    /// A target on `backend` that passed no recorded choice over.
    fn on(backend: &str) -> CalendarTarget {
        CalendarTarget {
            backend: backend.to_owned(),
            passed_over: None,
        }
    }

    #[test]
    fn the_target_is_the_connected_provider_that_writes_a_calendar() {
        let only = target(write(
            &registry(),
            &["strava", "mock_calendar_b"],
            &[],
            None,
        ));
        assert_eq!(only, on("mock_calendar_b"));
    }

    #[test]
    fn several_calendars_and_nothing_choosing_ask_the_athlete_in_ranking_order() {
        let registry = registry();
        for connected in [
            ["mock_calendar_b", "mock_calendar_a"],
            ["mock_calendar_a", "mock_calendar_b"],
        ] {
            let asked = write(&registry, &connected, &[], None).expect("resolved");
            assert_eq!(
                asked,
                CalendarResolution::Choose {
                    calendars: vec!["mock_calendar_a".to_owned(), "mock_calendar_b".to_owned()],
                    passed_over: None,
                },
                "never a guess among {connected:?}"
            );
        }
    }

    #[test]
    fn the_calendar_the_ledger_already_writes_is_kept_without_asking() {
        // An athlete pushing to one calendar who connects a second one keeps
        // landing on the first: their entries live there.
        let kept = target(write(
            &registry(),
            &["mock_calendar_a", "mock_calendar_b"],
            &["mock_calendar_b"],
            None,
        ));
        assert_eq!(kept, on("mock_calendar_b"));
        let unconnected = write(
            &registry(),
            &["mock_calendar_a", "mock_calendar_b"],
            &["mock_read_only"],
            None,
        )
        .expect("resolved");
        assert!(
            matches!(unconnected, CalendarResolution::Choose { .. }),
            "a ledger on no qualifying connection decides nothing: {unconnected:?}"
        );
    }

    #[test]
    fn the_athletes_choice_wins_over_the_ledger_and_the_ranking() {
        let chosen = target(resolve_calendar_target(
            &registry(),
            &CalendarCandidates {
                connected: &["mock_calendar_a", "mock_calendar_b"],
                live: &["mock_calendar_a"],
                preferred: Some("mock_calendar_b"),
            },
            None,
        ));
        assert_eq!(chosen, on("mock_calendar_b"));
    }

    #[test]
    fn a_choice_off_the_connections_is_passed_over_and_reported() {
        let registry = registry();
        // Disconnected: the rules beneath it decide, and it is reported.
        let kept = target(resolve_calendar_target(
            &registry,
            &CalendarCandidates {
                connected: &["mock_calendar_a", "mock_calendar_b"],
                live: &["mock_calendar_a"],
                preferred: Some("wahoo"),
            },
            None,
        ));
        assert_eq!(kept.backend, "mock_calendar_a");
        assert_eq!(kept.passed_over.as_deref(), Some("wahoo"));
        // Connected but writing no calendar: the same.
        let only = target(resolve_calendar_target(
            &registry,
            &CalendarCandidates {
                connected: &["mock_read_only", "mock_calendar_b"],
                live: &[],
                preferred: Some("mock_read_only"),
            },
            None,
        ));
        assert_eq!(only.backend, "mock_calendar_b");
        assert_eq!(only.passed_over.as_deref(), Some("mock_read_only"));
        // Several left and nothing choosing: asked, with the choice reported.
        let asked = resolve_calendar_target(
            &registry,
            &CalendarCandidates {
                connected: &["mock_calendar_a", "mock_calendar_b"],
                live: &[],
                preferred: Some("wahoo"),
            },
            None,
        )
        .expect("resolved");
        assert_eq!(
            asked,
            CalendarResolution::Choose {
                calendars: vec!["mock_calendar_a".to_owned(), "mock_calendar_b".to_owned()],
                passed_over: Some("wahoo".to_owned()),
            }
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
    fn a_push_only_calendar_is_offered_after_a_calendar_provider_beside_it() {
        let registry = garmin_pair();
        let asked = write(&registry, &["garmin", "mock_calendar_b"], &[], None).expect("resolved");
        assert_eq!(
            asked,
            CalendarResolution::Choose {
                calendars: vec!["mock_calendar_b".to_owned(), "garmin".to_owned()],
                passed_over: None,
            }
        );
        let alone = target(write(&registry, &["garmin"], &[], None));
        assert_eq!(alone.backend, "garmin");
        let pushed = target(write(
            &registry,
            &["garmin", "mock_calendar_b"],
            &["garmin"],
            None,
        ));
        assert_eq!(
            pushed.backend, "garmin",
            "the ledger's calendar still wins over the ranking"
        );
    }

    #[test]
    fn a_choice_recorded_against_a_mirror_lands_on_its_writing_connection() {
        let chosen = target(resolve_calendar_target(
            &garmin_pair(),
            &CalendarCandidates {
                connected: &["sciotte_garmin", "garmin", "mock_calendar_a"],
                live: &["mock_calendar_a"],
                preferred: Some("sciotte_garmin"),
            },
            None,
        ));
        assert_eq!(chosen, on("garmin"));
        assert_eq!(
            chosen_calendar(
                &garmin_pair(),
                &["sciotte_garmin", "garmin"],
                Some("sciotte_garmin")
            ),
            Some("garmin")
        );
    }

    #[test]
    fn a_named_mirror_writes_through_the_api_connection_of_the_same_name() {
        let named = target(write(
            &garmin_pair(),
            &["sciotte_garmin", "garmin"],
            &[],
            Some("sciotte_garmin"),
        ));
        assert_eq!(named, on("garmin"));
    }

    #[test]
    fn a_named_mirror_alone_is_refused_with_the_connection_that_writes() {
        let err = write(
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
    fn a_named_provider_wins_over_the_choice_and_the_ledger() {
        let named = target(resolve_calendar_target(
            &registry(),
            &CalendarCandidates {
                connected: &["mock_calendar_a", "mock_calendar_b"],
                live: &["mock_calendar_a"],
                preferred: Some("mock_calendar_a"),
            },
            Some("mock_calendar_b"),
        ));
        assert_eq!(named, on("mock_calendar_b"));
    }

    #[test]
    fn a_named_provider_without_the_capability_is_refused_by_name() {
        let err = write(&registry(), &[], &[], Some("mock_read_only")).unwrap_err();
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
        let err = write(&registry(), &["strava", "mock_read_only"], &[], None).unwrap_err();
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
        assert!(!writes_a_calendar(
            &registry(),
            &["strava", "mock_read_only"]
        ));
        assert!(writes_a_calendar(
            &registry(),
            &["strava", "mock_calendar_b"]
        ));
    }

    fn read(registry: &ProviderRegistry, connected: &[&str], live: &[&str]) -> AppResult<String> {
        calendar_read_target(
            registry,
            &CalendarCandidates {
                connected,
                live,
                preferred: None,
            },
        )
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
        let target = read(&registry, &["strava"], &[]);
        assert_eq!(target.unwrap(), expected);
    }

    #[test]
    fn a_read_finds_a_disconnected_calendar_the_ledger_still_holds() {
        let target = read(&registry(), &["strava"], &["mock_calendar_b"]);
        assert_eq!(target.unwrap(), "mock_calendar_b");
    }

    #[test]
    fn a_read_prefers_the_connected_target() {
        let target = read(&registry(), &["mock_calendar_b"], &[]);
        assert_eq!(target.unwrap(), "mock_calendar_b");
    }

    #[test]
    fn a_read_follows_the_choice_then_the_ledger_then_the_ranking_and_never_asks() {
        let registry = registry();
        let both = ["mock_calendar_a", "mock_calendar_b"];
        assert_eq!(read(&registry, &both, &[]).unwrap(), "mock_calendar_a");
        assert_eq!(
            read(&registry, &both, &["mock_calendar_b"]).unwrap(),
            "mock_calendar_b"
        );
        let chosen = calendar_read_target(
            &registry,
            &CalendarCandidates {
                connected: &both,
                live: &["mock_calendar_a"],
                preferred: Some("mock_calendar_b"),
            },
        );
        assert_eq!(chosen.unwrap(), "mock_calendar_b");
    }
}
