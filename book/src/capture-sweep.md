# Capture Staleness & the Nightly Refresh

A provider capture can stop without failing. On 2026-08-28 an athlete's sciotte
capture stopped; it was still stopped two days later. Three real activities never
landed, the agent kept answering confidently from a training log that had frozen,
and nothing anywhere said so. It was found by hand (carnet#149).

Two fields an operator would have watched were recording something other than
what their names suggest:

| Field | Reads as | Actually means |
|---|---|---|
| `cached_activities.synced_at` | "data was refreshed then" | "some code wrote these rows then" |
| `provider_connections.status` | "the connection works" | "nobody has proven it broken" |

Serving a window *out of* the cache used to write those rows back, moving
`synced_at` forward — the cache refreshed its own freshness, so it could never go
stale and could never alarm (`176ab975c` removed that mask). And a scrape-backed
connection has no token to expire, so nothing ever tested it and its status
stayed `active`.

The honest column is `activity_fetch_freshness.fetched_at`, which advances only
when a fetch genuinely reached a provider.

## The signal is a divergence, not an age

`provider_connections.last_used_at` is touched at the serve chokepoint on **every**
serve, including one the durable cache answered. `fetched_at` moves only on a
live fetch. So:

> recently used **and** long since fetched = this athlete is being served from
> cache by a provider that has stopped answering

Requiring recent use is what keeps the check quiet. An athlete who has not opened
the app has nothing that should have fetched, so their old `fetched_at` is
correct rather than alarming, and they are excluded from the judged population
entirely.

## The two halves

Both run in one lane, `Monitor: Capture Staleness (dev)`
(`.github/workflows/monitor-capture-staleness.yml`), daily at 12:17 UTC. It
refreshes first, then reads.

### Refresh — `POST /admin/diagnostics/capture-refresh`

`crates/pierre-tool-runtime/src/capture_sweep.rs`. Walks every live connection in
the same snapshot the reader judges and refreshes its recent head through the
platform's own fetch path.

**It never attempts a login.** That is load-bearing, not fastidiousness: a fresh
scraper login can demand a 2FA phone tap inside a four-minute window, and the
scraper service scales to zero holding no durable session, so there is nothing to
resume from after an idle period. An unattended re-login is not something that
can be made to work at 04:00.

So the sweep refreshes only what is already authenticated. When a fetch fails in
an auth-shaped way — `AppError::provider_auth_required`, which both a lapsed
scrape session and a 401 raise — the connection is flipped to `needs_reauth` with
reason `session_expired` and the sweep moves on. Every other failure is recorded
and otherwise ignored: a flake is not a disconnect.

The athlete's next turn already consults that flag and hands back a reconnect
link, so the failure mode converts from *"silently captures nothing for days"*
into *"tells you it needs a reconnect"*. Nothing is pushed from the sweep itself;
waking someone at 4am to say a scrape session lapsed is the opposite of what this
buys.

Two properties worth knowing:

- Fetches go through `activity_fetch::fetch_provider_head` — the same
  write-through a chat turn uses, including the freshness mark. A second writer
  to the activity cache would duplicate the upsert, dedup, prune and freshness
  logic, and that mark is the exact signal the reader trusts.
- It runs **inline** on the request, not detached: the service scales to zero, so
  a spawned task has no CPU once the response is sent. Bounded at 90 s per
  connection and 480 s overall (under the API's 600 s request timeout), and a
  sweep that runs out reports `completed: false` plus a
  `skipped_budget_exhausted` line per connection it never reached.

### Read — `GET /admin/diagnostics/capture-staleness`

`ActivityCacheRepository::capture_freshness_snapshot` pairs every **active**
connection with its last successful fetch, cross-tenant — an operator asking "has
any athlete's capture stopped" cannot ask it one tenant at a time. It returns no
verdict; `partition_stale_captures` applies the thresholds, which are clamped to
`1..=720` hours and echoed back so the caller sees which question was answered.

Connections needing re-auth are excluded from the snapshot: they have a known
reason to have stopped fetching and already surface through the reconnect path.
That is also why the refresh drops a connection it flags — the reader stops
counting a failure that now has a stated remedy.

The lane **warns rather than reds** and files a carnet issue, because a red on a
schedule-only lane would have `monitor-scheduled-lanes.yml` report the monitor as
broken at the moment it is working.

## OAuth and scrape-backed are not the same job

| | OAuth providers | Scrape-backed |
|---|---|---|
| Examples | Strava, Fitbit, WHOOP, Intervals.icu | `sciotte`, `sciotte_garmin` |
| Credential | a token with an expiry, plus a refresh token | a browser session; no token, no expiry |
| Is it valid? | readable before doing any work | **only discoverable by trying** — the attempt *is* the test |
| How it gets flagged | the token-refresh path flags a dead grant on its own | the sweep's fast auth-shaped failure sets the flag |

The scrape-backed half cannot be pre-filtered, so it costs one attempt per
athlete per night rather than one per *valid* athlete.

## Running it by hand

```bash
# Refresh every live capture (needs manage_configuration)
curl -X POST -H "Authorization: Bearer ${ADMIN_TOKEN}" \
  "${BASE_URL}/admin/diagnostics/capture-refresh"

# What the honest capture clock says (needs view_configuration)
curl -H "Authorization: Bearer ${ADMIN_TOKEN}" \
  "${BASE_URL}/admin/diagnostics/capture-staleness?stale_after_hours=24&active_within_hours=48"
```

Both responses carry ids only, never an email: this repo is public and the
monitor echoes them into world-readable CI logs.
