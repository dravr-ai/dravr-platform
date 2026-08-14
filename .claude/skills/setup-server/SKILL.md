---
name: setup-server
description: Bootstrap the Pierre dev stack — database, seeds, admin/test users, server, frontend, Expo — for development and testing
user-invocable: true
---

# Setup Server

Brings the Pierre dev environment from zero to fully running with seeded test data.

**CLAUDE: when invoked as `/setup-server`, run:**

```bash
./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh
```

This is the only supported entry point. It resets the database, runs migrations, creates the
admin, runs every seeder, and starts all services. There is no "skip the wipe" mode — for a
stack that is already up and healthy, just leave it alone.

> **Destructive.** It kills every running service and recreates the dev DB. If someone may be
> mid-session on this worktree, confirm before running.

## Flags

| Flag | Effect |
|---|---|
| *(none)* | debug build — faster to compile, slower to run |
| `--release` | optimized build — slower to compile, faster to drive |
| `--native` | build the native mobile dev client instead of Expo Go |
| `--stream-logs` | tail service logs in the foreground after startup |
| `--tunnel` | start a Cloudflare tunnel and rewrite `BASE_URL` for physical-device testing |

## What it starts

| Service | Port | Log |
|---|---|---|
| Pierre server | 8081 | `logs/pierre-server.log` |
| Vite web frontend | 5173 | `logs/frontend.log` |
| Expo / Metro | 8082 | `logs/expo.log` |
| Dev fixture API (serves seeded Strava/Garmin activities) | 9555 | `logs/fixture.log` |

Port 8081 is reserved for Pierre — never start anything else on it. Expo must stay on 8082.

Verify before using the stack:

```bash
curl -sf http://localhost:8081/health && echo " server ok"
curl -sf -o /dev/null http://localhost:5173 && echo " vite ok"
```

## Prerequisites

- `.envrc` present and populated (`cp .envrc.example .envrc && direnv allow`). The script
  hard-fails listing any missing required var — `DATABASE_URL`,
  `PIERRE_MASTER_ENCRYPTION_KEY`, and the seven `PIERRE_SCIOTTE_*` backpressure vars.
- A coach source at `../dravr-contremaitre/prompts/coaches`, or `PIERRE_COACHES_DIR` set.
  The script exits if it is absent — coach definitions live in dravr-contremaitre as the
  single source of truth.

## Credentials

There are **two different kinds** here, and conflating them is what has repeatedly produced
wrong docs:

**The admin is environment-dependent.** The setup script resolves
`${ADMIN_EMAIL:-admin@example.com}` / `${ADMIN_PASSWORD:-AdminPassword123}`, so whatever
`.envrc` sets wins. On a machine whose `.envrc` sets `ADMIN_EMAIL="admin@pierre.mcp"`, that
is the operator account and `admin@example.com` does not exist at all — logging in with the
default returns `invalid_grant` and looks like a broken server. **Read your own `.envrc`
before assuming.** CI, which has no `.envrc` override, gets the defaults.

The admin is created with `--super-admin` on purpose: `cookie_admin_middleware` derives
console permissions from the role, so a plain `admin` silently loses contremaitre config,
store moderation and impersonation.

**The seeded users are constants**, baked into `crates/pierre-seeders/src/demo_data.rs` —
`webtest@pierre.dev`, `mobiletest@pierre.dev`, `alice@acme.com`, `bob@startup.io`. Read the
seeder for their passwords rather than trusting a restated table: that table has been copied
into docs, hooks and skills and drifted (the session banner advertised a mobile password the
seeder never produced, and this script's summary printed a `bob@acme.com` that does not
exist).

`frontend/e2e-real/seeded-credentials.real.spec.ts` pins both kinds against a live server —
the seeded users by constant, the admin via `ADMIN_EMAIL`/`ADMIN_PASSWORD` from the
environment. Run it under a sourced `.envrc`, or the admin case falls back to the default and
fails for the wrong reason.

Admin API token is written to `logs/admin-token.txt` at the end of the run.

## Admin users and tokens (`pierre-cli`)

| Command | Purpose |
|---|---|
| `cargo run --bin pierre-cli -- user create --email <e> --password <p>` | create a user |
| `cargo run --bin pierre-cli -- token generate --service <s> --expires-days 30` | API token |
| `cargo run --bin pierre-cli -- token generate --service admin_console --super-admin` | super-admin token, no expiry |
| `cargo run --bin pierre-cli -- token list --detailed` | list admin tokens |
| `cargo run --bin pierre-cli -- token revoke <token_id>` | revoke |

## Seeders

The setup script runs all of these already. Run one individually only against an existing
database you do not want to wipe.

| Subcommand | Creates |
|---|---|
| `pierre-cli seed bootstrap` | admin + demo users (idempotent) |
| `pierre-cli seed coaches --coaches-dir <dir>` | coach personas from contremaitre markdown |
| `pierre-cli seed demo-data --days 30` | demo users, dashboard analytics, API keys, usage series |
| `pierre-cli seed social` | friend connections, shared insights, reactions, feed |
| `pierre-cli seed mobility` | stretches, yoga poses, activity-muscle mappings |
| `pierre-cli seed llm-usage --admin-email <e> --days 30` | LLM call records for analytics |
| `pierre-cli seed synthetic-activities` | activities + a fixture-backed provider connection |
| `pierre-cli seed insight-samples` | validates insight sample markdown (no DB writes) |

### synthetic-activities

Seeds activities **and** an encrypted dev-fixture `oauth_token`, so the user appears as a
real Strava or Garmin athlete and the activities are served back through the genuine provider
code path by the fixture API on 9555.

```bash
pierre-cli seed synthetic-activities --email <e> --provider strava --count 30 --days 30
```

| Arg | Default | Notes |
|---|---|---|
| `--email` | `user@example.com` | the CLI default is *not* a seeded account — always pass a real one |
| `--count` | 100 | |
| `--days` | 90 | |
| `--provider` | `strava` | `strava` \| `garmin` |
| `--reset` | off | clear existing synthetic activities first |
| `--seed` | none | fix the RNG for reproducible data |

Demo users are seeded as fixture-backed Strava/Garmin athletes rather than as a bare
`synthetic` provider, so they clear the onboarding provider gate and get real coach
recommendations.

## Manual service control

| Command | Effect |
|---|---|
| `./bin/start-server.sh` | start Pierre alone |
| `./bin/stop-server.sh` | stop the whole dev stack; `--server-only` stops just the backend |
| `./bin/stop-all.sh` | stop every service the setup script started |
| `./bin/dev-logs.sh` | tail dev logs |

## Troubleshooting

**Server won't start**

```bash
lsof -i :8081        # what is holding the port
./bin/stop-all.sh
./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh
```

**Migration checksum mismatch** — an applied sqlx migration was edited, which panics at boot.
Never edit an applied migration; add a new one. Recover by re-running the setup script, which
recreates the database from scratch.

**"Port 8081 in use" while starting Expo** — that is Pierre running correctly. Expo belongs
on 8082 (`bun start` in `frontend-mobile/` is already configured for it).

**Token expired** — Strava tokens last 6 hours; the server auto-refreshes from the stored
refresh token. If refresh fails the user must re-run the OAuth flow.

## Related

- `test-web-app` — drive the running stack end-to-end and land regression tests
- `validate-frontend` / `validate-mobile` / `validate-sdk`
- `create-worktree` — isolated worktree on its own ports
