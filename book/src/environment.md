<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Copyright (c) 2026 dravr.ai -->

# Environment Configuration

Environment variables for Pierre Fitness Platform. Copy `.envrc.example` to `.envrc` and customize.

## Setup

```bash
cp .envrc.example .envrc
# edit .envrc with your settings
direnv allow  # or: source .envrc
```

## Required Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `DATABASE_URL` | Database connection string | `sqlite:./data/users.db` |
| `PIERRE_MASTER_ENCRYPTION_KEY` | Master encryption key (base64) | `openssl rand -base64 32` |

## Server Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `HTTP_PORT` | `8081` | Server port |
| `RUST_LOG` | `info` | Log level (debug, info, warn, error) |
| `JWT_EXPIRY_HOURS` | `24` | JWT token expiration |
| `PIERRE_RSA_KEY_SIZE` | `4096` | RSA key size (2048 for dev, 4096 for prod) |

## Database

### SQLite (Development)

```bash
export DATABASE_URL="sqlite:./data/users.db"
```

### PostgreSQL (Production)

```bash
export DATABASE_URL="postgresql://user:pass@localhost/pierre_db"
export POSTGRES_MAX_CONNECTIONS="10"
export POSTGRES_MIN_CONNECTIONS="0"
export POSTGRES_ACQUIRE_TIMEOUT="30"
```

### SQLx Pool Configuration

Fine-tune database connection pool behavior:

| Variable | Default | Description |
|----------|---------|-------------|
| `SQLX_IDLE_TIMEOUT_SECS` | `600` | Seconds before idle connections are closed |
| `SQLX_MAX_LIFETIME_SECS` | `1800` | Maximum connection lifetime in seconds |
| `SQLX_TEST_BEFORE_ACQUIRE` | `true` | Validate connections before use |
| `SQLX_STATEMENT_CACHE_CAPACITY` | `100` | Prepared statement cache size |

## Tokio Runtime Configuration

Configure the async runtime for performance tuning:

| Variable | Default | Description |
|----------|---------|-------------|
| `TOKIO_WORKER_THREADS` | CPU cores | Number of worker threads |
| `TOKIO_THREAD_STACK_SIZE` | OS default | Thread stack size in bytes |
| `TOKIO_THREAD_NAME` | `pierre-worker` | Worker thread name prefix |

## Provider Configuration

### Default Provider

```bash
export PIERRE_DEFAULT_PROVIDER=strava  # strava, garmin, fitbit, whoop, coros, synthetic
```

### Strava

```bash
# required for strava oauth
export PIERRE_STRAVA_CLIENT_ID=your-client-id
export PIERRE_STRAVA_CLIENT_SECRET=your-client-secret
```

### Garmin

```bash
# required for garmin oauth
export PIERRE_GARMIN_CLIENT_ID=your-consumer-key
export PIERRE_GARMIN_CLIENT_SECRET=your-consumer-secret
```

### Fitbit

```bash
export PIERRE_FITBIT_CLIENT_ID=your-client-id
export PIERRE_FITBIT_CLIENT_SECRET=your-client-secret
```

### WHOOP

```bash
export PIERRE_WHOOP_CLIENT_ID=your-client-id
export PIERRE_WHOOP_CLIENT_SECRET=your-client-secret
```

### COROS

```bash
# required for coros oauth (apply for API access first)
export PIERRE_COROS_CLIENT_ID=your-client-id
export PIERRE_COROS_CLIENT_SECRET=your-client-secret
```

**Note:** Redirect URIs are automatically constructed by the server based on the `HOST` and `HTTP_PORT` configuration. No `*_REDIRECT_URI` environment variables are needed.

**Note:** COROS API documentation is private. Apply at [COROS Developer Portal](https://support.coros.com/hc/en-us/articles/17085887816340).

### Terra (150+ Wearables)

```bash
export TERRA_API_KEY=your-api-key
export TERRA_DEV_ID=your-dev-id
export TERRA_WEBHOOK_SECRET=your-webhook-secret
```

### Synthetic (No Credentials Needed)

```bash
export PIERRE_DEFAULT_PROVIDER=synthetic
# no oauth credentials required - works out of the box
```

## Algorithm Configuration

Configure fitness calculation algorithms via environment variables.

| Variable | Default | Options |
|----------|---------|---------|
| `PIERRE_MAXHR_ALGORITHM` | `tanaka` | fox, tanaka, nes, gulati |
| `PIERRE_TRIMP_ALGORITHM` | `hybrid` | bannister_male, bannister_female, edwards_simplified, lucia_banded, hybrid |
| `PIERRE_TSS_ALGORITHM` | `avg_power` | avg_power, normalized_power, hybrid |
| `PIERRE_VDOT_ALGORITHM` | `daniels` | daniels, riegel, hybrid |
| `PIERRE_TRAINING_LOAD_ALGORITHM` | `ema` | ema, sma, wma, kalman |
| `PIERRE_RECOVERY_ALGORITHM` | `weighted` | weighted, additive, multiplicative, minmax, neural |
| `PIERRE_FTP_ALGORITHM` | `from_vo2max` | 20min_test, 8min_test, ramp_test, from_vo2max, hybrid |
| `PIERRE_LTHR_ALGORITHM` | `from_maxhr` | from_maxhr, from_30min, from_race, lab_test, hybrid |
| `PIERRE_VO2MAX_ALGORITHM` | `from_vdot` | from_vdot, cooper, rockport, astrand, bruce, hybrid |

See [configuration.md](configuration.md#algorithm-configuration) for algorithm details.

## Fitness Configuration

### Effort Thresholds (1-10 Scale)

```bash
export FITNESS_EFFORT_LIGHT_MAX="3.0"
export FITNESS_EFFORT_MODERATE_MAX="5.0"
export FITNESS_EFFORT_HARD_MAX="7.0"
# > 7.0 = very_high
```

### Heart Rate Zone Thresholds (% of Max HR)

```bash
export FITNESS_ZONE_RECOVERY_MAX="60.0"
export FITNESS_ZONE_ENDURANCE_MAX="70.0"
export FITNESS_ZONE_TEMPO_MAX="80.0"
export FITNESS_ZONE_THRESHOLD_MAX="90.0"
# > 90.0 = vo2max
```

### Personal Records

```bash
export FITNESS_PR_PACE_IMPROVEMENT_THRESHOLD="5.0"
```

## Weather Integration

```bash
export OPENWEATHER_API_KEY="your-api-key"
export FITNESS_WEATHER_ENABLED="true"
export FITNESS_WEATHER_WIND_THRESHOLD="15.0"
export FITNESS_WEATHER_CACHE_DURATION_HOURS="24"
export FITNESS_WEATHER_REQUEST_TIMEOUT_SECONDS="10"
export FITNESS_WEATHER_RATE_LIMIT_PER_MINUTE="60"
```

## LLM Configuration

### Provider Selection

```bash
# Active LLM provider
# Valid values: gemini, groq, local, ollama, vllm, localai,
#               claude_code, claude-code, copilot, github_copilot,
#               copilot_sdk, cursor_agent, opencode, cli
export PIERRE_LLM_PROVIDER=gemini
```

### API-Based Provider Models

```bash
# Model for Gemini and Groq (required when using those providers)
export PIERRE_LLM_MODEL=gemini-2.5-flash

# Primary model used by LlmModelConfig (same value as PIERRE_LLM_MODEL in most setups)
export PIERRE_LLM_DEFAULT_MODEL=gemini-2.5-flash

# Gemini API key
export GEMINI_API_KEY=your-gemini-api-key

# Groq API key
export GROQ_API_KEY=your-groq-api-key

# Local LLM (Ollama, vLLM, LocalAI)
export LOCAL_LLM_BASE_URL=http://localhost:11434/v1
export LOCAL_LLM_MODEL=qwen2.5:14b-instruct
export LOCAL_LLM_API_KEY=  # leave empty for Ollama
```

### Gemini Retry Configuration

```bash
export GEMINI_MAX_RETRIES=5
export GEMINI_INITIAL_RETRY_DELAY_MS=2000
export GEMINI_MAX_RETRY_DELAY_MS=30000
```

### Fallback Configuration

When the primary provider fails or is rate-limited, Pierre can automatically switch to a fallback provider.

```bash
# Enable automatic fallback (disabled by default)
export PIERRE_LLM_FALLBACK_ENABLED=true

# Fallback provider and model
export PIERRE_LLM_FALLBACK_PROVIDER=gemini
export PIERRE_LLM_FALLBACK_MODEL=gemini-2.5-pro

# Seconds to wait before activating fallback (default: 10)
export PIERRE_LLM_FALLBACK_WAIT_SECS=10
```

### CLI and SDK Provider Configuration

These providers require no API key — they use authentication from the installed CLI tool.

```bash
# Model override for Copilot SDK (default: claude-opus-4.6)
export COPILOT_SDK_MODEL=claude-opus-4.6

# Model override for CLI runners (Claude Code, Copilot CLI, Cursor Agent, OpenCode)
export CLI_LLM_MODEL=claude-opus-4.6

# Override binary path (skip automatic which-discovery)
export CLI_LLM_BINARY=/usr/local/bin/claude

# Timeout per LLM subprocess call in seconds (default: 120)
export CLI_LLM_TIMEOUT_SECS=120

# Comma-separated extra arguments passed to the CLI binary
export CLI_LLM_EXTRA_ARGS=

# Working directory for subprocess execution (default: current directory)
export CLI_LLM_WORKING_DIR=
```

See [LLM Provider Integration](llm-providers.md) for full provider documentation including the three-way tool loop dispatch and all supported models.

## Rate Limiting

```bash
export RATE_LIMIT_ENABLED="true"
export RATE_LIMIT_REQUESTS="100"
export RATE_LIMIT_WINDOW="60"  # seconds
```

## Cache Configuration

```bash
export CACHE_MAX_ENTRIES="10000"
export CACHE_CLEANUP_INTERVAL_SECS="300"
export REDIS_URL="redis://localhost:6379"  # optional, uses in-memory if not set
```

## Backup Configuration

```bash
export BACKUP_ENABLED="true"
export BACKUP_INTERVAL="21600"  # 6 hours in seconds
export BACKUP_RETENTION="7"      # days
export BACKUP_DIRECTORY="./backups"
```

## Activity Limits

```bash
export MAX_ACTIVITIES_FETCH="100"
export DEFAULT_ACTIVITIES_LIMIT="20"
```

## OAuth Callback

```bash
export OAUTH_CALLBACK_PORT="35535"  # bridge callback port for focus recovery
```

## Development Defaults

For dev/test only (leave empty in production):

```bash
# Regular user defaults (for OAuth login form)
export OAUTH_DEFAULT_EMAIL="user@example.com"
export OAUTH_DEFAULT_PASSWORD="userpass123"

# Admin user defaults (for setup scripts)
export ADMIN_EMAIL="admin@pierre.mcp"
export ADMIN_PASSWORD="adminpass123"
```

## Frontend Configuration

```bash
export VITE_BACKEND_URL="http://localhost:8081"
```

## Production vs Development

| Setting | Development | Production |
|---------|-------------|------------|
| `DATABASE_URL` | sqlite | postgresql |
| `PIERRE_RSA_KEY_SIZE` | 2048 | 4096 |
| `RUST_LOG` | debug | info |
| Redirect URIs | http://localhost:... | https://... |
| `OAUTH_DEFAULT_*` | set | empty |

## Security Notes

- Never commit `.envrc` (gitignored)
- Use HTTPS redirect URIs in production
- Generate unique `PIERRE_MASTER_ENCRYPTION_KEY` per environment
- Rotate provider credentials periodically
