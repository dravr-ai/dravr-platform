# ABOUTME: Multi-stage production Dockerfile for Pierre MCP Server (Rust binary)
# ABOUTME: Uses cargo-chef for dependency caching; runtime stage uses debian-slim with non-root user

# Stage 0: base image with build dependencies and cargo-chef installed
FROM --platform=linux/amd64 rust:1.96-slim AS chef

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
# The private dravr-* git dependencies authenticate through a global gitconfig
# that rewrites https://github.com/ to an x-access-token URL. HOME (/root) is a
# tmpfs mount for the duration of the RUN, so that rendered config — which holds
# the token in plaintext — lives in memory and never enters the stage snapshot.
# Persisting it would publish the token: buildx exports every stage of this
# build to a registry buildcache tag (cache-to mode=max) whose read access is
# granted project-wide. The credential has to stay a real file rather than move
# to git's GIT_CONFIG_COUNT/KEY/VALUE environment form, because this build
# leaves CARGO_NET_GIT_FETCH_WITH_CLI unset and cargo's libgit2 fetch path reads
# config from files only.
RUN --mount=type=secret,id=git_token \
    --mount=type=tmpfs,target=/root \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_token)@github.com/".insteadOf "https://github.com/" \
    && cargo chef prepare --recipe-path recipe.json

# Stage 2: build all workspace dependencies (cached unless Cargo.toml/Cargo.lock changes)
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Credential handling as in the planner stage: HOME is a tmpfs mount so the
# rendered gitconfig never reaches this stage's layers.
RUN --mount=type=secret,id=git_token \
    --mount=type=tmpfs,target=/root \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_token)@github.com/".insteadOf "https://github.com/" \
    && cargo chef cook --release --no-default-features --features "server-production,postgresql,sqlx/migrate,telemetry,gcp-kms" --recipe-path recipe.json

# Build the server binary and pierre-cli (which now hosts all seeder subcommands).
# Reruns only when source code changes.
COPY . .
# Credential handling as in the planner stage: HOME is a tmpfs mount so the
# rendered gitconfig never reaches this stage's layers.
RUN --mount=type=secret,id=git_token \
    --mount=type=tmpfs,target=/root \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_token)@github.com/".insteadOf "https://github.com/" \
    && cargo build --release --no-default-features --features "server-production,postgresql,sqlx/migrate,telemetry,gcp-kms" \
    --bin pierre-mcp-server \
    --bin pierre-cli

# Verify binaries were built
RUN ls -la target/release/pierre-mcp-server target/release/pierre-cli

FROM --platform=linux/amd64 debian:trixie-slim

# Cache-bust token for the apt layer below. The build uses a registry-backed
# buildx cache, so the `apt-get upgrade` layer is otherwise reused verbatim
# across builds even after Debian ships a security update — leaving patched
# packages (notably chromium, which renders untrusted web content during
# sciotte scraping) stale in the image. Bump this date whenever Trivy flags a
# fixed base-package CVE to force a fresh `apt-get update && upgrade` that pulls
# the patched version (e.g. chromium 150.0.7871.46-1~deb13u1 closes the
# CVE-2026-13774..13785 / 14090..14120 batch).
# Also bump it when a pulled package is itself broken: 150.0.7871.46-1~deb13u1
# SIGTRAPs on every headless launch (Debian packaging bug), killing sciotte
# credential login; 150.0.7871.100-1~deb13u1 fixes it.
# chromium 151.0.7922.108-1~deb13u1 closes CVE-2026-19155, a sandbox escape via
# use-after-free in Payments that gated the dev deploy on 2026-08-10.
# chromium 151.0.7922.137-1~deb13u1 closes CVE-2026-19556..19560 — four
# arbitrary-code-execution use-after-frees (V8, Blink, HTML, extensions) plus a
# TabStrip sandbox escape — which gated the dev deploy on 2026-08-13.
# util-linux 2.41.5-0+deb13u1 closes CVE-2026-53615, an integer overflow in
# libblkid's DOS partition parser (libblkid/src/partitions/dos.c). Trivy reports
# it once per binary package the source ships — bsdutils, libblkid1,
# liblastlog2-2, libmount1, libsmartcols1, libuuid1, login, mount, util-linux —
# so it reads as nine HIGH findings from one defect. trixie itself still carries
# the vulnerable 1:2.41-5 and only trixie-security has the fix, which is why the
# upgrade below is what pulls it rather than a newer base-image tag. Gated the
# dev deploy on 2026-08-17.
# chromium 151.0.7922.169-1~deb13u1 closes CVE-2026-76033..76052 — 22 HIGH
# findings in one source package, among them a site-isolation bypass, arbitrary
# code execution in Dawn and V8, and information disclosure. Gated the dev
# deploy on 2026-08-21, on a commit that touched no Docker and no dependency:
# the buildx registry cache was still serving the 137 layer.
ARG APT_SECURITY_EPOCH=2026-08-21

# Upgrade all system packages to patch CVEs, then install runtime dependencies
# nodejs + npm: needed for copilot CLI and claude-code CLI (LLM providers via embacle)
# git: required by copilot CLI subprocess
# chromium + libs: required by dravr-sciotte (headless Chrome scraping via chromiumoxide)
RUN echo "apt security epoch: ${APT_SECURITY_EPOCH}" \
    && apt-get update && apt-get upgrade -y \
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
# COPILOT_CLI_VERSION must be >=1.0.59: that is the first release whose
# `initialize` advertises `agentCapabilities.mcpCapabilities {http,sse}`, which
# the native MCP tool-calling bridge (COPILOT_HEADLESS_MCP_TOOL_CALLING) relies
# on. Running an older CLI (1.0.44) under native MCP + Autopilot crashed the
# `copilot --acp` subprocess mid-synthesis ("connection closed unexpectedly")
# after tool calls succeeded — the regression that started 2026-06-07 when the
# bridge was enabled while the image still pinned 1.0.44.
#
# - copilot_headless LLM provider (primary): GitHub Copilot CLI in ACP mode
# - claude_code LLM provider (runtime fallback): Anthropic Claude Code CLI
#   authenticated via CLAUDE_CODE_OAUTH_TOKEN (one-year subscription token from
#   `claude setup-token`, see PIERRE_LLM_FALLBACK_PROVIDER in dev/main.tf).
#
# After install: remove npm and its transitive deps (node-tar CVEs) since npm
# is not needed at runtime.
ARG COPILOT_CLI_VERSION=1.0.59
ARG CLAUDE_CODE_CLI_VERSION=2.1.163
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