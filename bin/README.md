# bin/ - Runtime Scripts

Day-to-day scripts for running Pierre development environment.

## Prerequisites

All scripts require `.envrc` to be configured. Copy from example and edit:

```bash
cp .envrc.example .envrc
# Edit .envrc with your settings, then:
direnv allow  # or: source .envrc
```

### Required Environment Variables

Scripts will fail fast if these are missing:

| Variable | Description | Category |
|----------|-------------|----------|
| `DATABASE_URL` | Database connection string | **CRITICAL** |
| `PIERRE_MASTER_ENCRYPTION_KEY` | Master encryption key (base64) | **CRITICAL** |

Generate the encryption key with: `openssl rand -base64 32`

### Provider OAuth Variables (based on `PIERRE_DEFAULT_PROVIDER`)

| Provider | Required Variables |
|----------|-------------------|
| `strava` | `PIERRE_STRAVA_CLIENT_ID`, `PIERRE_STRAVA_CLIENT_SECRET` |
| `garmin` | `PIERRE_GARMIN_CLIENT_ID`, `PIERRE_GARMIN_CLIENT_SECRET` |
| `synthetic` | None (works out of the box) |

See `book/src/environment.md` for the complete variable reference.

## Quick Start

```bash
# Full setup: reset DB, seed all data, start all 3 servers
./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh
```

## Available Scripts

| Script | Description |
|--------|-------------|
| `setup-db-with-seeds-and-oauth-and-start-servers.sh` | **THE ONE SCRIPT** - Complete dev environment setup |
| `start-server.sh` | Start Pierre MCP server only (`HTTP_PORT`, default 8081) |
| `stop-server.sh` | Stop this checkout's dev stack (delegates to `stop-all.sh`); `--server-only` stops just this checkout's backend |
| `stop-all.sh` | Stop this checkout's server, fixture, Vite, Expo and tunnel, and reset a dead tunnel URL |
| `start-frontend.sh` | Start web frontend only (Vite's default port 5173) |
| `start-tunnel.sh` | Start Cloudflare tunnel for mobile testing; rewrites the `BASE_URL` line in `.envrc` and the `EXPO_PUBLIC_API_URL` line in `frontend-mobile/.env` |
| `stop-tunnel.sh` | Stop the tunnel and point `BASE_URL` back at the local server |

## What `setup-db-with-seeds-and-oauth-and-start-servers.sh` Does

1. Stops any running services
2. Resets database (backs up existing, runs fresh migrations)
3. Seeds all data:
   - Admin user (from `.envrc`: `ADMIN_EMAIL`, `ADMIN_PASSWORD`)
   - Agent personas from `../dravr-contremaitre/prompts/coaches` (21 today)
   - Demo users (Alice, Bob, etc.)
   - Visual test users (webtest, mobiletest)
   - Mobility data (stretches, yoga poses)
4. Starts Pierre MCP Server (`HTTP_PORT`, default 8081)
5. Starts Web Frontend (port 5173)
6. Starts Expo Mobile (port 8082)
7. Generates admin API token
8. Displays summary with credentials, URLs, and log paths

## Log Files

After running the setup script, logs are available at:

```bash
tail -f logs/pierre-server.log  # Pierre MCP Server
tail -f logs/frontend.log       # Web Frontend
tail -f logs/expo.log           # Expo Mobile
tail -f logs/*.log              # All logs
```

## Stopping Services

```bash
./bin/stop-server.sh                # the whole stack: server, fixture, Vite, Expo, tunnel
./bin/stop-server.sh --server-only  # only the backend, to simulate an outage
```

Both are scoped to this checkout: they stop the processes these scripts recorded
in `logs/*.pid` and leave another worktree's stack running. Several worktrees of
this repo run side by side, and a name — `pierre-mcp-server`, `expo start` —
matches every one of them at once.

A port still held by a process this checkout can prove is its own (a stack
started before the pid files existed) is reclaimed as well; a port held by
another checkout is named, with its pid and directory, and left alone.

Ownership is the pid file, so a service that never wrote one is invisible to
these scripts and is reached only through its port — and a Cloudflare tunnel
holds no local port. A `cloudflared` these scripts started is stopped by
`./bin/stop-tunnel.sh`, which also points `BASE_URL` back at the local server.

One started outside them has neither a pid file nor a port, so nothing here can
find it. Read the process list, confirm the tunnel is the one you mean, and kill
that pid: `ps -eo pid,args | grep '[c]loudflared tunnel'`. `pkill -f 'cloudflared
tunnel'` reaches every checkout's tunnel on the machine, which on a host running
several worktrees takes down someone else's device testing.

## See Also

- `scripts/` - CI/Dev tools (validation, testing, release)
- `scripts/setup/setup-claude-code-mcp.sh` - Claude Code session JWT setup
