// ABOUTME: How long an Overpass walk waits on a silent instance before also asking the next:
// ABOUTME: short while an instance failed lately, calm otherwise so mirrors stay unasked
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long the walk waits on an instance that has not answered before it
/// also asks the next one, once an instance has failed within the last
/// [`OVERPASS_TROUBLE_MEMORY`].
///
/// A hedge, not a race: an instance that answers within it is the only one
/// asked, and no instance ever has two requests in flight, as the main
/// instance's policy asks. On 2026-11-27 overpass-api.de and
/// overpass.private.coffee hung while maps.mail.ru answered in 5-7 s; the
/// old walk gave each instance 5 s in turn, so the one that answered was
/// never reached. A failure, unlike silence, asks the next one at once.
pub(super) const OVERPASS_HEDGE_DELAY: Duration = Duration::from_secs(1);

/// The hedge while no instance has failed within the last
/// [`OVERPASS_TROUBLE_MEMORY`].
///
/// A healthy main instance answers in 2-10 s, so a 1 s hedge would ask the
/// volunteer mirrors on most calls of a calm day and add to the load whose
/// saturation is the outage. Calm, the main instance answers alone unless it
/// is slower than this. Within the budget a calm walk reaches two instances,
/// not three: the first walk of an outage may fail, and the walks after it
/// hedge at [`OVERPASS_HEDGE_DELAY`] for [`OVERPASS_TROUBLE_MEMORY`].
pub(super) const OVERPASS_CALM_HEDGE_DELAY: Duration = Duration::from_secs(4);

/// How long after an instance's last failure the walks keep the short hedge.
///
/// Longer than the cooldown on purpose: an outage outlasts a cooldown, and
/// a walk that went calm again each time the cooldowns ran out would let the
/// hung instances spend its budget, failing one search every cooldown. The
/// record of a failure stays until the instance answers again.
const OVERPASS_TROUBLE_MEMORY: Duration = Duration::from_mins(30);

/// Whether one of `endpoints` failed within [`OVERPASS_TROUBLE_MEMORY`].
/// `cooldowns` holds, per endpoint, its last failure plus `cooldown`.
pub(super) fn troubled(
    endpoints: &[String],
    cooldowns: &HashMap<String, Instant>,
    now: Instant,
    cooldown: Duration,
) -> bool {
    endpoints.iter().any(|endpoint| {
        cooldowns
            .get(endpoint)
            .and_then(|until| until.checked_sub(cooldown))
            .is_some_and(|failed| failed + OVERPASS_TROUBLE_MEMORY > now)
    })
}

/// The hedge a walk runs under.
pub(super) const fn delay(troubled: bool, short: Duration, calm: Duration) -> Duration {
    if troubled {
        short
    } else {
        calm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_keeps_the_short_hedge_past_its_cooldown() {
        let endpoints = vec!["https://a/i".to_owned()];
        let cooldown = Duration::from_mins(5);
        let now = Instant::now() + Duration::from_hours(1);
        let failed_ago = |secs| now.checked_sub(Duration::from_secs(secs)).unwrap() + cooldown;
        let mut cooldowns = HashMap::new();
        // Failed 10 minutes ago: its cooldown ran out, the trouble did not.
        cooldowns.insert(endpoints[0].clone(), failed_ago(600));
        assert!(
            troubled(&endpoints, &cooldowns, now, cooldown),
            "bug: an expired cooldown made the walk calm during an outage"
        );
        // Failed 40 minutes ago: calm again.
        cooldowns.insert(endpoints[0].clone(), failed_ago(2400));
        assert!(
            !troubled(&endpoints, &cooldowns, now, cooldown),
            "bug: a failure older than the trouble memory still hedges early"
        );
        assert!(
            !troubled(&endpoints, &HashMap::new(), now, cooldown),
            "bug: no failure on record is trouble"
        );
    }
}
