// ABOUTME: The sciotte read services a server calls, and the one a session's id picks among them
// ABOUTME: Every call naming a session reaches one instance, so the session meets one cookie jar (carnet#1504)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A sciotte read service runs one instance: a read imports the session,
//! reads, then exports the copy a rotation left, and the instance's cookie
//! jar holds the provider's rotated cookie between reads. Reads scale by
//! running several such services and sending every call that names a session
//! to the one its id picks ([`SciotteReads::for_session`]).
//!
//! The pick is a jump consistent hash (Lamping & Veach) of the id's SHA-256,
//! so every API instance computes the same service for a session, and adding
//! a service moves about one session in `N + 1` to it.

use std::fmt;
use std::sync::Arc;

use dravr_sciotte::client::SciotteClient;
use ring::digest::{self, SHA256};
use tracing::{error, info};

use super::factory;

/// The multiplier of the jump consistent hash's linear congruential step.
const JUMP_MULTIPLIER: u64 = 2_862_933_555_777_941_757;

/// The sciotte read services this server calls, in their configured order.
///
/// Never empty: a server with no read service holds no `SciotteReads`.
#[derive(Clone)]
pub struct SciotteReads {
    /// Shared, not copied: every factory, provider and health reader holds a
    /// clone of the one list the server built at startup.
    clients: Arc<[SciotteClient]>,
}

impl fmt::Debug for SciotteReads {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteReads")
            .field("services", &self.clients.len())
            .finish()
    }
}

impl SciotteReads {
    /// The client of the read service the session `session_id` names.
    ///
    /// Every call naming the session — its import, its reads, the export of
    /// a rotation — must go through this one client: the session's jar lives
    /// on that service's instance, and the client keeps the note of a rotation
    /// its next export answers.
    ///
    /// With more than one service it logs the one picked (`sciotte_service`),
    /// which is how an operator sees a session stay on its service.
    #[must_use]
    pub fn for_session(&self, session_id: &str) -> &SciotteClient {
        let index = self.index_for(session_id);
        if self.clients.len() > 1 {
            info!(
                sciotte_service = index,
                "sciotte call routed to its read service"
            );
        }
        &self.clients[index]
    }

    /// The position, in the configured order, of the read service the session
    /// `session_id` names.
    fn index_for(&self, session_id: &str) -> usize {
        service_index(session_id, self.clients.len())
    }

    /// The first read service: where sign-ins run when no sign-in service of
    /// their own is configured.
    #[must_use]
    pub(crate) fn primary(&self) -> &SciotteClient {
        &self.clients[0]
    }

    /// Whether the session `session_id` names the first read service, the one
    /// a sign-in runs on when no sign-in service of its own is configured.
    #[must_use]
    pub(crate) fn names_primary(&self, session_id: &str) -> bool {
        self.index_for(session_id) == 0
    }

    /// Every read service, in order: a disconnect drops the session from all
    /// of them, since a sign-in or a change in their number can leave a copy
    /// on one the session no longer names.
    #[must_use]
    pub fn all(&self) -> &[SciotteClient] {
        &self.clients
    }
}

/// The read services `urls` names, each built like
/// [`startup_client`](factory::startup_client) with the shared `audience`.
///
/// `None` when no URL is given, or — logged here — when any of them cannot
/// be built: dropping one would move every session to another service, so a
/// broken entry turns the read services off instead.
#[must_use]
pub fn startup_reads(urls: &[String], audience: Option<&str>) -> Option<SciotteReads> {
    if urls.is_empty() {
        return None;
    }
    let mut clients = Vec::with_capacity(urls.len());
    for (index, url) in urls.iter().enumerate() {
        let Some(client) = factory::startup_client(url, audience) else {
            error!(
                read_service = index,
                "a sciotte read service could not be built; every read service is turned off \
                 rather than moving its sessions to another"
            );
            return None;
        };
        clients.push(client);
    }
    Some(SciotteReads {
        clients: clients.into(),
    })
}

/// The service, among `services`, that `session_id` names: a jump consistent
/// hash of the first eight bytes of the id's SHA-256, read big-endian.
fn service_index(session_id: &str, services: usize) -> usize {
    let hash = digest::digest(&SHA256, session_id.as_bytes());
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&hash.as_ref()[..8]);
    let mut key = u64::from_be_bytes(prefix);
    let services = u128::try_from(services).unwrap_or(u128::MAX);
    let mut bucket: u128 = 0;
    let mut next: u128 = 0;
    while next < services {
        bucket = next;
        key = key.wrapping_mul(JUMP_MULTIPLIER).wrapping_add(1);
        next = (bucket + 1) * (1_u128 << 31) / (u128::from(key >> 33) + 1);
    }
    usize::try_from(bucket).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Session ids whose service is pinned at two read services. The suites
    /// `sciotte_session_rotation_test` (pierre-providers) and
    /// `scrape_health_sync_test` (pierre-server) read these same ids, one per
    /// service.
    const ON_SERVICE_0: &str = "shard-session-f";
    const ON_SERVICE_1: &str = "shard-session-b";

    #[test]
    fn one_service_takes_every_session() {
        for id in [ON_SERVICE_0, ON_SERVICE_1, "", "anything"] {
            assert_eq!(
                service_index(id, 1),
                0,
                "bug: the only service did not take {id:?}"
            );
        }
    }

    /// The indexes are pinned on purpose: a session's service is a routing
    /// value every API instance and every deployed revision must compute
    /// alike, so a change to the hash moves live sessions off the instance
    /// holding their cookie jar.
    #[test]
    fn the_pinned_ids_name_their_service() {
        assert_eq!(
            service_index(ON_SERVICE_0, 2),
            0,
            "bug: {ON_SERVICE_0} left the first of two services"
        );
        assert_eq!(
            service_index(ON_SERVICE_1, 2),
            1,
            "bug: {ON_SERVICE_1} left the second of two services"
        );
        for (id, service) in [
            ("shard-session-g", 0),
            ("shard-session-c", 1),
            (ON_SERVICE_0, 2),
            (ON_SERVICE_1, 2),
        ] {
            assert_eq!(
                service_index(id, 3),
                service,
                "bug: {id} no longer names service {service} of three"
            );
        }
    }

    #[test]
    fn a_third_service_takes_sessions_only_onto_itself() {
        for n in 0..1_000 {
            let id = format!("session-{n}");
            let before = service_index(&id, 2);
            let after = service_index(&id, 3);
            assert!(
                after == before || after == 2,
                "{id} moved from {before} to {after}, not to the new service"
            );
        }
    }

    #[test]
    fn two_services_share_sessions_evenly() {
        let on_first = (0..1_000)
            .filter(|n| service_index(&format!("session-{n}"), 2) == 0)
            .count();
        assert!(
            (450..=550).contains(&on_first),
            "{on_first} of 1000 fixed ids on the first service"
        );
    }

    #[test]
    fn no_url_and_an_unbuildable_url_build_no_read_services() {
        assert!(startup_reads(&[], None).is_none(), "no URL, no service");
        // Off loopback with no audience: the client refuses to build.
        let urls = vec![
            "http://127.0.0.1:9".to_owned(),
            "https://sciotte-read-1.example.run.app".to_owned(),
        ];
        assert!(
            startup_reads(&urls, None).is_none(),
            "one unbuildable service turns every read service off"
        );
    }

    #[test]
    fn the_services_keep_their_order() {
        let urls = vec![
            "http://127.0.0.1:9".to_owned(),
            "http://127.0.0.1:10".to_owned(),
        ];
        let reads = startup_reads(&urls, None).expect("loopback services build");
        assert_eq!(reads.all().len(), 2, "both services are held");
        assert_eq!(
            reads.index_for(ON_SERVICE_1),
            1,
            "the pinned id names the second"
        );
        assert!(
            reads.names_primary(ON_SERVICE_0),
            "a session on the first service keeps the sign-in's copy there"
        );
        assert!(
            !reads.names_primary(ON_SERVICE_1),
            "a session on the second service has no read meet a copy on the first"
        );
    }
}
