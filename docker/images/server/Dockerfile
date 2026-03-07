# ABOUTME: Multi-stage production Dockerfile for Pierre MCP Server (Rust binary)
# ABOUTME: Uses cargo-chef for workspace-aware dependency caching; runtime stage uses debian-slim with non-root user

# Stage 0: base image with build dependencies and cargo-chef installed
FROM --platform=linux/amd64 rust:1.92-slim AS chef

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-chef --locked

WORKDIR /app

# Stage 1: generate dependency recipe from the full workspace source tree
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: build all workspace dependencies (cached unless Cargo.toml/Cargo.lock changes)
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Build the actual application (reruns only when source code changes)
COPY . .
RUN cargo build --release --bin pierre-mcp-server

# Verify the binary was built
RUN ls -la target/release/pierre-mcp-server

FROM --platform=linux/amd64 debian:trixie-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    sqlite3 \
    curl \
    bash \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create app user and directories
RUN useradd -m -u 1001 appuser \
    && mkdir -p /app/data \
    && chown -R appuser:appuser /app

# Copy binary and entrypoint script from builder stage
COPY --from=builder /app/target/release/pierre-mcp-server /app/pierre-mcp-server
COPY docker/images/server/entrypoint.sh /app/docker-entrypoint.sh

# Make binary and entrypoint executable and owned by appuser
RUN chmod +x /app/pierre-mcp-server \
    && chmod +x /app/docker-entrypoint.sh \
    && chown appuser:appuser /app/pierre-mcp-server \
    && chown appuser:appuser /app/docker-entrypoint.sh

# Switch to non-root user
USER appuser

# Set working directory
WORKDIR /app

# Set environment variables with secure defaults
ENV RUST_LOG=info
ENV DATABASE_URL=sqlite:./data/users.db
ENV ENCRYPTION_KEY_PATH=./data/encryption.key
ENV JWT_SECRET_PATH=./data/jwt.secret
ENV MCP_PORT=8080
ENV HTTP_PORT=8081

# Expose ports
EXPOSE 8080 8081

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8081/health || exit 1

# Use entrypoint script for environment loading
ENTRYPOINT ["/app/docker-entrypoint.sh"]

# Default command
CMD ["/app/pierre-mcp-server"]