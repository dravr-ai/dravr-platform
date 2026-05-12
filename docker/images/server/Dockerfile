# ABOUTME: Multi-stage production Dockerfile for Pierre MCP Server (Rust binary)
# ABOUTME: Uses cargo-chef for dependency caching; runtime stage uses debian-slim with non-root user

# Stage 0: base image with build dependencies and cargo-chef installed
FROM --platform=linux/amd64 rust:1.94-slim AS chef

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ca-certificates \
    git \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-chef --locked

WORKDIR /app

# Stage 1: generate dependency recipe from the full workspace source tree
FROM chef AS planner
COPY . .
RUN --mount=type=secret,id=git_token \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_token)@github.com/".insteadOf "https://github.com/" \
    && cargo chef prepare --recipe-path recipe.json

# Stage 2: build all workspace dependencies (cached unless Cargo.toml/Cargo.lock changes)
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=secret,id=git_token \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_token)@github.com/".insteadOf "https://github.com/" \
    && cargo chef cook --release --no-default-features --features "server-production,postgresql,sqlx/migrate" --recipe-path recipe.json

# Build the server binary and pierre-cli (which now hosts all seeder subcommands).
# Reruns only when source code changes.
COPY . .
RUN --mount=type=secret,id=git_token \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_token)@github.com/".insteadOf "https://github.com/" \
    && cargo build --release --no-default-features --features "server-production,postgresql,sqlx/migrate" \
    --bin pierre-mcp-server \
    --bin pierre-cli

# Verify binaries were built
RUN ls -la target/release/pierre-mcp-server target/release/pierre-cli

FROM --platform=linux/amd64 debian:trixie-slim

# Upgrade all system packages to patch CVEs, then install runtime dependencies
# nodejs + npm: needed for copilot CLI and claude-code CLI (LLM providers via embacle)
# git: required by copilot CLI subprocess
# chromium + libs: required by dravr-sciotte (headless Chrome scraping via chromiumoxide)
RUN apt-get update && apt-get upgrade -y \
    && apt-get install -y --no-install-recommends \
    ca-certificates \
    sqlite3 \
    curl \
    bash \
    git \
    nodejs \
    npm \
    chromium \
    && rm -rf /var/lib/apt/lists/*

# Install GitHub Copilot CLI and Anthropic Claude Code CLI in one layer so the
# npm strip below covers both. Pinned versions for reproducible builds; bump
# deliberately after validating model compatibility (e.g. claude-opus-4.7
# requires Copilot CLI >=1.0.30; CLAUDE_CODE_OAUTH_TOKEN auth requires
# claude-code >=2.x).
#
# - copilot_headless LLM provider (primary): GitHub Copilot CLI in ACP mode
# - claude_code LLM provider (runtime fallback): Anthropic Claude Code CLI
#   authenticated via CLAUDE_CODE_OAUTH_TOKEN (one-year subscription token from
#   `claude setup-token`, see PIERRE_LLM_FALLBACK_PROVIDER in dev/main.tf).
#
# After install: remove npm and its transitive deps (node-tar CVEs) since npm
# is not needed at runtime.
ARG COPILOT_CLI_VERSION=1.0.44
ARG CLAUDE_CODE_CLI_VERSION=2.1.139
RUN npm install -g "@github/copilot-linux-x64@${COPILOT_CLI_VERSION}" \
    && npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_CLI_VERSION}" \
    && NPM_PREFIX="$(npm prefix -g)" \
    && ln -sf "$NPM_PREFIX/bin/copilot-linux-x64" /usr/local/bin/copilot \
    && copilot --version \
    && claude --version \
    && rm -rf "$NPM_PREFIX/lib/node_modules/npm" "$NPM_PREFIX/bin/npm" "$NPM_PREFIX/bin/npx"

# Create app user and directories
RUN useradd -m -u 1001 appuser \
    && mkdir -p /app/data \
    && chown -R appuser:appuser /app

# Copy server binary, pierre-cli (seeders + admin tools), and entrypoint scripts
COPY --from=builder /app/target/release/pierre-mcp-server /app/pierre-mcp-server
COPY --from=builder /app/target/release/pierre-cli /app/pierre-cli
COPY --from=builder /app/commands /app/commands
# Eval harness golden fixtures — admin /admin/evals/fixtures browser walks
# this directory at runtime. PIERRE_EVALS_FIXTURES_DIR (set below) points
# the runtime at this baked-in path so the route returns real data instead
# of the empty fallback when the workspace tree isn't present.
COPY --from=builder /app/crates/pierre-evals/fixtures /app/fixtures
COPY docker/images/server/entrypoint.sh /app/docker-entrypoint.sh
COPY docker/images/server/seed-entrypoint.sh /app/seed-entrypoint.sh
COPY docker/images/server/chromium-headless.sh /usr/local/bin/chromium-headless

# Make binaries and entrypoints executable and owned by appuser
RUN chmod +x /app/pierre-mcp-server \
    && chmod +x /app/pierre-cli \
    && chmod +x /app/docker-entrypoint.sh \
    && chmod +x /app/seed-entrypoint.sh \
    && chmod +x /usr/local/bin/chromium-headless \
    && chown -R appuser:appuser /app

# Switch to non-root user
USER appuser

# Set working directory
WORKDIR /app

# Git commit SHA baked into the image at build time (survives Terraform reconciliation)
ARG GIT_COMMIT_SHA=unknown
ENV GIT_COMMIT_SHA=${GIT_COMMIT_SHA}

# Server runtime configuration
ENV RUST_LOG=info
ENV ENCRYPTION_KEY_PATH=./data/encryption.key
ENV JWT_SECRET_PATH=./data/jwt.secret
ENV MCP_PORT=8080
ENV HTTP_PORT=8081
# Eval harness fixtures location (see COPY above). The route resolves
# this env var first; absent or pointing at a missing dir falls back to
# the workspace-relative default and logs a single warn (no Slack page).
ENV PIERRE_EVALS_FIXTURES_DIR=/app/fixtures
# chromiumoxide auto-detection uses CHROME env var; point to our headless wrapper
# which injects --ozone-platform=headless before delegating to the real binary
ENV CHROME=/usr/local/bin/chromium-headless

# Expose ports
EXPOSE 8080 8081

# Health check (Cloud Run liveness probe)
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8081/health || exit 1

# Use entrypoint script for environment loading
ENTRYPOINT ["/app/docker-entrypoint.sh"]

# Default command
CMD ["/app/pierre-mcp-server"]