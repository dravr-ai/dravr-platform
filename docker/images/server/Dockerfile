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
    && cargo chef cook --release --features "postgresql,client-messaging" --recipe-path recipe.json

# Build the actual application and seeder binaries (reruns only when source code changes)
COPY . .
RUN --mount=type=secret,id=git_token \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_token)@github.com/".insteadOf "https://github.com/" \
    && cargo build --release --features "postgresql,client-messaging" \
    --bin pierre-mcp-server \
    --bin seed-bootstrap \
    --bin seed-coaches \
    --bin seed-mobility \
    --bin seed-synthetic-activities

# Verify binaries were built
RUN ls -la target/release/pierre-mcp-server \
    target/release/seed-bootstrap \
    target/release/seed-coaches \
    target/release/seed-mobility \
    target/release/seed-synthetic-activities

FROM --platform=linux/amd64 debian:trixie-slim

# Upgrade all system packages to patch CVEs, then install runtime dependencies
# nodejs + npm: needed for copilot CLI (copilot_headless LLM provider via embacle)
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

# Install GitHub Copilot CLI (required for copilot_headless LLM provider via embacle)
# After install: remove npm and its transitive deps (node-tar CVEs) since npm is not needed at runtime
RUN npm install -g @github/copilot-linux-x64 \
    && NPM_PREFIX="$(npm prefix -g)" \
    && ln -sf "$NPM_PREFIX/bin/copilot-linux-x64" /usr/local/bin/copilot \
    && copilot --version \
    && rm -rf "$NPM_PREFIX/lib/node_modules/npm" "$NPM_PREFIX/bin/npm" "$NPM_PREFIX/bin/npx"

# Create app user and directories
RUN useradd -m -u 1001 appuser \
    && mkdir -p /app/data \
    && chown -R appuser:appuser /app

# Copy server binary, seeder binaries, and entrypoint scripts from builder stage
COPY --from=builder /app/target/release/pierre-mcp-server /app/pierre-mcp-server
COPY --from=builder /app/target/release/seed-bootstrap /app/seed-bootstrap
COPY --from=builder /app/target/release/seed-coaches /app/seed-coaches
COPY --from=builder /app/target/release/seed-mobility /app/seed-mobility
COPY --from=builder /app/target/release/seed-synthetic-activities /app/seed-synthetic-activities
COPY --from=builder /app/coaches /app/coaches
COPY docker/images/server/entrypoint.sh /app/docker-entrypoint.sh
COPY docker/images/server/seed-entrypoint.sh /app/seed-entrypoint.sh

# Make binaries and entrypoints executable and owned by appuser
RUN chmod +x /app/pierre-mcp-server \
    && chmod +x /app/seed-bootstrap \
    && chmod +x /app/seed-coaches \
    && chmod +x /app/seed-mobility \
    && chmod +x /app/seed-synthetic-activities \
    && chmod +x /app/docker-entrypoint.sh \
    && chmod +x /app/seed-entrypoint.sh \
    && chown -R appuser:appuser /app

# Switch to non-root user
USER appuser

# Set working directory
WORKDIR /app

# Server runtime configuration
ENV RUST_LOG=info
ENV ENCRYPTION_KEY_PATH=./data/encryption.key
ENV JWT_SECRET_PATH=./data/jwt.secret
ENV MCP_PORT=8080
ENV HTTP_PORT=8081
# chromiumoxide auto-detection uses CHROME env var to locate the browser binary
ENV CHROME=/usr/bin/chromium
# Headless Chromium in a container has no X server or display; force the Ozone
# headless platform so Chromium does not try to connect to X11 or D-Bus.
ENV CHROMIUM_FLAGS="--ozone-platform=headless --disable-gpu --disable-software-rasterizer"

# Expose ports
EXPOSE 8080 8081

# Health check (Cloud Run liveness probe)
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8081/health || exit 1

# Use entrypoint script for environment loading
ENTRYPOINT ["/app/docker-entrypoint.sh"]

# Default command
CMD ["/app/pierre-mcp-server"]