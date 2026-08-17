# Commitment Sweep

An athlete says "three easy runs this week". The coach confirms it. A week later
the platform counts what they actually recorded and tells them.

That last step is the part most coaching products skip. Dravr already had the
harder half — a reinforcement loop that reads real activity data and decides
whether a recommendation worked — but its verdict only ever reached a playbook
counter. The commitment sweep is the loop the athlete is actually part of.

## What a commitment is

A `Commitment` is the athlete's own promise: a **count**, a **sport** (or any
sport), and a **window**. It is deliberately distinct from its three siblings in
`pierre-memory`:

| Entity | Whose | Verified against data? | Has a due date? |
|---|---|---|---|
| `TrainingPlan` | the coach's prescription | no | per-week structure |
| `CoachFollowup` | the coach's promise to check in | no | yes |
| `Playbook` | a learned trigger → intervention pattern | yes, in aggregate | no |
| **`Commitment`** | **the athlete's own** | **yes, per row** | **yes** |

It is the only one of the four counted against what actually happened.

## It is never inferred

The sibling advice-capture path extracts coach recommendations from a turn with
an LLM. A commitment is not captured that way, and the reason is the whole
entity: post-hoc extraction cannot tell

> "I'll run three times this week"

from a bare

> "ok"

said to the coach's suggestion — and only the first is something the athlete
would recognise as a promise. Reporting on the second is presumptuous, and being
told you failed at something you never agreed to is the fastest way to lose an
athlete's trust in the coach.

So the row is written by an explicit tool call, `commitment_create`, whose
description requires the athlete to have named both a count and a deadline. If
they only assented, the coach's instruction is to ask them to confirm both and
call the tool on that answer. Requiring the athlete to restate the *when* is also
the mechanism the implementation-intentions literature credits for the adherence
effect in the first place — the confirming turn is not overhead, it is the
active ingredient.

## The loop

```
commitment_create        window closes         hourly tick          hourly tick
      │                        │                    │                    │
   ┌──▼──┐                     │              ┌─────▼─────┐        ┌─────▼─────┐
   │ open├─────────────────────┴─────────────►│  labeled  ├───────►│ reported  │
   └──┬──┘                                    └─────┬─────┘        └───────────┘
      │                                             │
      │ commitment_cancel                           │ no route for 7 days
      ▼                                             ▼
  cancelled                                     expired
```

`status` and `reported_at` are separate transitions on purpose. The sibling
`coach_followups` table overloads one `delivered` state to mean both "rendered
into a prompt" and "pushed to the athlete", which lets a chat turn and the
scheduler consume the same row out from under each other. Here the sweep moves
`open → labeled` and only the reporter moves `labeled → reported`.

## Counting

The sweep reads the athlete's cached activities in `[window_start, window_end]`,
keeps the ones matching the commitment's sport, and counts **sessions**, not
rows. Two activity rows whose start times fall within ten minutes of each other
are one session: an athlete with both Strava and Garmin connected has the same
morning run cached twice, and reporting three runs for a week that held two is
worse than saying nothing.

The verdict is one of three:

| Outcome | Condition |
|---|---|
| `met` | completed ≥ target |
| `partial` | 0 < completed < target |
| `missed` | completed == 0 |

`partial` is first-class because two of three is the most common real result and
the only one worth a conversation. The pre-existing `OutcomeLabel` has no such
variant — its middle case is `Neutral`, which is explicitly non-reinforcing —
which is why a commitment carries its own verdict enum rather than borrowing it.

## The two guards

Both exist to decide when *not* to speak.

**Freshness.** The activity cache is warmed write-through by chat and tool
fetches; nothing back-fills it on a schedule. An athlete who promised something
and then never opened the app has an empty window that means "we do not know",
not "they did nothing". A shortfall is therefore only labeled once the cache has
been synced past the window's end. Otherwise the row defers and retries. (A
commitment whose data never arrives expires unlabeled — see
`LIMITATION(registre#32)`.)

**Cadence.** At most one verdict per athlete per 24 hours, and at most one per
tick. A message for every missed commitment is a reason to mute the coach; the
accountability value is in being noticed, not in being scolded. A verdict held by
the cap stays `labeled` and goes out on a later tick.

## Delivery is per channel

Proactive messaging is not uniformly available, so the reporter resolves the
messaging session that owns the originating conversation and applies a policy:

| Channel | Unsolicited message |
|---|---|
| Telegram, Slack, Discord | any time — the bot may message a user who started it |
| WhatsApp, Messenger | only inside Meta's 24-hour re-engagement window |
| App push | any time, subject to the athlete's own notification preferences |
| Web chat (no session) | falls back to app push |

Outside the Meta window a plain text send is rejected outright (error `131047`),
and the platform's channel adapters implement no template payload, so the verdict
is **held, not forced and not dropped**. The window reopens the moment the athlete
speaks again. The outbound retry queue is deliberately not used for this: its
backoff is seconds against a 24-hour window, so an enqueued out-of-window send is
a guaranteed dead-letter dressed up as a retry.

A push suppressed by quiet hours or a disabled category is also held rather than
burned — the athlete did not see it, so it has not been delivered.

## What the athlete reads

The message is composed from the verdict's numbers and the sanitized sport slug
through the localized string registry. It never includes the stored statement,
and never anything a provider supplied.

That is not stylistic. The sweep reads activity data, a tainted source — 26 tools
carry `UNTRUSTED_OUTPUT` — and then sends outbound, which is exactly the
exfiltration shape the Guardian hard-denies at the tool chokepoint. A background
service does not pass through that chokepoint, so the discipline has to be
structural instead: an activity titled with an injection payload can move a
count, and moving a count is all it can ever do.

## Windows are civil, not UTC

`commitment_create` takes `due_date` as a `YYYY-MM-DD` date in the athlete's own
calendar and resolves it once, at creation, to the UTC instant of their local
end-of-day. "This week" is a civil-calendar claim; resolving it in UTC would tell
an athlete in Auckland they missed a Sunday run at lunchtime on Sunday.
Timezone-less accounts fall back to UTC.

## Operator surface

| | |
|---|---|
| Table | `athlete_commitments` (both migration trees) |
| Repository | `CommitmentRepository` |
| Tools | `commitment_create`, `commitment_cancel` (category `memory`) |
| Prompt block | `## Commitments the athlete made`, prompt-assembly stage 7f.1 |
| Sweep cadence | hourly; `PIERRE_COMMITMENT_SWEEP_INTERVAL_SECS` overrides |
| Batch size | 50 rows per pass |
| Notify events | `commitment.created`, `commitment.swept`, `commitment.reported` |

The sweep is skipped entirely for in-memory databases, like every sibling
background worker, so test servers never run it.
