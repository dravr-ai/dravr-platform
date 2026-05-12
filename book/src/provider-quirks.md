# Provider Quirks & Footguns

Documented gotchas from the third-party fitness and LLM APIs Dravr integrates
with. Each entry names a symptom, the underlying cause, and the mitigation
that's already in the code (with a file pointer so the next person who hits
the same symptom doesn't repeat the debug session).

The rule for adding an entry: if a vendor's documented behavior is *the
opposite* of what you'd assume from the parameter name, OR if a failure mode
took more than two hours to debug, write it down here.

## Strava

### `/athlete/activities` flips to ascending order when only `after` is set

**Symptom:** Snapshot pipeline misses an athlete's freshest activities even
though the Strava connection is healthy and the API returns 200. The most
recent activity Pierre sees lags actual reality by weeks. Manifests as the
group bot reporting *"je n'ai pas une ride GPS de 200+ km affichée ici"*
when the user did, in fact, ride 234 km two days ago.

**Cause:** Strava's `/athlete/activities` endpoint sorts results
**ascending** (oldest first within the window) when only `after` is
supplied. With both `before` and `after` it switches back to **descending**
(newest first). With neither, also descending. The behavior is not in the
documentation prose; it surfaces from community reports and direct
observation. So a snapshot call with `after = now − 60d` and `per_page = 20`
returns the 20 *oldest* activities in the window — for active athletes
(20+ activities in 60 days), page 1 covers days 1–N and the newest rides
are at page 2+ which we never fetch.

**Mitigation:**
[`crates/pierre-providers/src/strava_provider.rs::get_activities_with_params`](https://github.com/dravr-ai/dravr-platform/blob/main/crates/pierre-providers/src/strava_provider.rs)
backstops a `None` `before` with `chrono::Utc::now().timestamp()` before
hitting Strava. This forces DESC ordering universally regardless of which
time filter the caller supplied.

**Don't:** Try to "fix" this by bumping the page limit or removing `after`.
The right contract for callers is "give me the latest N activities up to
this lower bound" and that requires DESC ordering at the provider call.

### Sciotte (Strava-mirror scraper) emits midnight-UTC timestamps

**Symptom:** Cross-provider dedup leaves the same workout in the snapshot
twice — once from `strava` with the real start time, once from `sciotte`
with a `00:00:00` UTC timestamp. Weekly volume and the `Recent:` block
double-count.

**Cause:** The scraper resolves activities at calendar-day precision only;
it has no access to the actual workout start time. The default dedup uses
a 15-minute time window which a 12-hour mismatch trivially blows past.

**Mitigation:**
[`crates/pierre-server/src/services/group_fitness.rs`](https://github.com/dravr-ai/dravr-platform/blob/main/crates/pierre-server/src/services/group_fitness.rs)
`is_likely_duplicate` falls back to calendar-date matching when either
side's timestamp is exactly midnight UTC. Helper:
`is_date_only_timestamp`.

## GitHub Copilot

### Session token refresh shares the `api.github.com` per-token rate limit

**Symptom:** Mid-shift, every chat turn fails with
`RPC error: {"code":-32000,"message":"Authentication required"}` even
though boot pre-check passed an hour ago, no secret was rotated, and the
binary is healthy. `/health/llm` reports green up until the first user
turn breaks.

**Cause (suspected):** The Copilot CLI subprocess holds a short-lived
session JWT it refreshes against
`api.github.com/copilot_internal/v2/token` using the `ghu_*` OAuth
token. That endpoint counts against the standard 5,000 req/hr per-token
limit on `api.github.com`. When the underlying GitHub user account is
hammered by other tooling (heavy `gh api` polling, GitHub Actions
running under the same user, etc.), the refresh exchange gets a 403 and
Copilot reports `Authentication required` to the ACP layer. The
underlying token is fine — it's just the refresh that's throttled.

**Mitigations in place:**
1. Runtime fallback chain (`PIERRE_LLM_RUNTIME_FALLBACK=true` +
   `PIERRE_LLM_FALLBACK_PROVIDER=gemini` in
   [`infra/environments/dev/main.tf`](https://github.com/dravr-ai/dravr-platform/blob/main/infra/environments/dev/main.tf))
   — Copilot transient failures fail over to Gemini per-request without
   restarting the container.
2. Real-roundtrip LLM probe
   ([`chat_provider_factory.rs::roundtrip_probe`](https://github.com/dravr-ai/dravr-platform/blob/main/crates/pierre-server/src/services/chat_provider_factory.rs))
   — the probe now sends a 1-token "ping" via `provider.complete()`
   rather than just checking the binary exists, so the next periodic
   tick catches a stale Copilot session within 5 minutes.

**Don't:** Rotate `COPILOT_GITHUB_TOKEN` as a first response to this
error class. The token is rarely the actual problem; check the fallback
status and the `gh api` quota first.

### Boot pre-check only validates the *prefix*, not the token

**Symptom:** A bad token boots a fresh pod, then every chat turn fails.

**Cause:** [`bin/pierre-mcp-server.rs::validate_copilot_token`](https://github.com/dravr-ai/dravr-platform/blob/main/crates/pierre-server/src/bin/pierre-mcp-server.rs)
inspects the prefix of `COPILOT_GITHUB_TOKEN` (`ghu_`, `gho_`,
`github_pat_`, etc.) to reject the obvious wrong-class tokens
(`ghp_` classic PATs, `ghr_` refresh tokens) at boot. It does **not**
call GitHub Copilot's API. A correctly-formatted but expired or revoked
token passes the pre-check and fails at first request.

**Mitigation:** The real-roundtrip probe above closes this gap once the
container has been up long enough for the first periodic tick.

## WHOOP

### Wrist-based misclassification: cycling reported as "Run"

**Symptom:** A long bike ride appears twice in the snapshot: once from
Strava as a `Ride` with full GPS distance, once from WHOOP as a `Run`
with no distance. Dedup rejects the pair because of the sport mismatch.

**Cause:** WHOOP's wrist motion classifier mistakes cycling cadence for
running cadence on certain riders or grip styles. The misclassified
"Run" carries valid duration and heart rate but no GPS distance, while
the GPS provider (Strava) records the same physical session as `Ride`.

**Mitigation:**
[`group_fitness.rs::is_cross_sport_duplicate`](https://github.com/dravr-ai/dravr-platform/blob/main/crates/pierre-server/src/services/group_fitness.rs)
collapses cross-provider pairs that disagree on sport when their
wall-clock windows overlap by at least
`CROSS_SPORT_OVERLAP_FRACTION` (60%) of the shorter session.
`pick_best` then keeps the GPS row (Strava) and drops the wrist-only
twin (WHOOP).

**Conservative cutoff:** Date-only timestamps (Sciotte) are excluded
from cross-sport dedup since the overlap math is unreliable without a
real start time.

## Adding a new entry

Template:

```
### <one-line symptom phrased as a user-facing or operator-facing failure>

**Symptom:** What you see in chat / logs / metrics when this fires.
**Cause:** The vendor behavior that's surprising. Cite docs or community
sources when available.
**Mitigation:** Repo file + function name. If there's no mitigation yet,
file an issue and link it here.
**Don't:** The wrong first response (token rotation, page bump, etc.).
```
