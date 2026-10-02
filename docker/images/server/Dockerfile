# ABOUTME: Multi-stage production Dockerfile for Pierre MCP Server (Rust binary)
# ABOUTME: Compiles with cargo-chef, or takes CI's prebuilt binaries; runtime stage is debian-slim, non-root

# Where the two binaries come from. `builder` (the default — docker-compose and
# any local `docker build`) compiles them in the stages below. `prebuilt` takes
# them from a `prebuilt-bin` named build context instead: publish-images.yml
# compiles on the runner against the GitHub-hosted rust-cache and passes
# `--build-context prebuilt-bin=<dir>`, so the CI image build never pulls the
# ~0.9 GB cargo-chef dependency layer back out of Artifact Registry — billed
# internet egress, because the runners sit outside GCP. Only the selected stage
# is built: with `prebuilt`, chef/planner/builder never run.
ARG BINARY_SOURCE=builder

# Stage 0: base image with build dependencies and cargo-chef installed
FROM --platform=linux/amd64 rust:1.98-slim AS chef
# embacle's copilot-sdk feature pulls github-copilot-sdk, whose build script
# downloads and embeds the Copilot CLI (~163 MB) unless told not to. Every
# workflow that runs cargo sets this in its env, .envrc.example sets it for
# dev shells, and this stage sets it for `cargo chef cook`, which runs before
# the tree is copied in.
ENV COPILOT_SKIP_CLI_DOWNLOAD=1

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
# Persisting it would publish the token to anything this stage is exported to:
# a registry buildcache with cache-to mode=max exports every stage of the build,
# and read access to the Artifact Registry cache packages is granted
# project-wide. The credential has to stay a real file rather than move
# to git's GIT_CONFIG_COUNT/KEY/VALUE environment form, because this build
# leaves CARGO_NET_GIT_FETCH_WITH_CLI unset and cargo's libgit2 fetch path reads
# config from files only.
RUN --mount=type=secret,id=git_token \
    --mount=type=tmpfs,target=/root \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_token)@github.com/".insteadOf "https://github.com/" \
    && cargo chef prepare --recipe-path recipe.json

# Stage 2: build all workspace dependencies (cached unless Cargo.toml/Cargo.lock changes)
FROM chef AS builder
# Both cargo invocations below use the release-lto profile (fat LTO) — the
# one release.yml tags binaries with — instead of the thin-LTO `--release`
# the image shipped until 2026-09-18. Fat LTO links the same code into a
# smaller binary, and ci-backend.yml's release-binary job builds with this
# exact invocation so the binary it sizes is the one that runs. carnet#471.
COPY --from=planner /app/recipe.json recipe.json
# Credential handling as in the planner stage: HOME is a tmpfs mount so the
# rendered gitconfig never reaches this stage's layers.
RUN --mount=type=secret,id=git_token \
    --mount=type=tmpfs,target=/root \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_token)@github.com/".insteadOf "https://github.com/" \
    && cargo chef cook --profile release-lto --no-default-features --features "server-production,postgresql,sqlx/migrate,telemetry,gcp-kms" --recipe-path recipe.json

# Build the server binary and pierre-cli (which now hosts all seeder subcommands).
# Reruns only when source code changes.
COPY . .
# Credential handling as in the planner stage: HOME is a tmpfs mount so the
# rendered gitconfig never reaches this stage's layers.
RUN --mount=type=secret,id=git_token \
    --mount=type=tmpfs,target=/root \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_token)@github.com/".insteadOf "https://github.com/" \
    && cargo build --profile release-lto --no-default-features --features "server-production,postgresql,sqlx/migrate,telemetry,gcp-kms" \
    --bin pierre-mcp-server \
    --bin pierre-cli

# Verify binaries were built
RUN ls -la target/release-lto/pierre-mcp-server target/release-lto/pierre-cli

# Stage 2b: the same two binaries, compiled outside Docker (BINARY_SOURCE=prebuilt).
# The `prebuilt-bin` context is a directory holding exactly pierre-mcp-server and
# pierre-cli, built with the builder stage's profile and features.
FROM scratch AS prebuilt
COPY --from=prebuilt-bin pierre-mcp-server pierre-cli /app/target/release-lto/

FROM ${BINARY_SOURCE} AS binaries

FROM --platform=linux/amd64 debian:trixie-slim

# Before the apt and npm layers, so it depends on the base image alone. Nothing
# after the large layers below may be a RUN: a RUN mounts its parent filesystem,
# which forces BuildKit to download every cached layer under it (~1 GB of
# chromium, node and the CLIs) from the registry cache on each build. The COPYs
# further down are --link with --chown/--chmod for the same reason.
RUN useradd -m -u 1001 appuser \
    && mkdir -p /app/data \
    && chown -R appuser:appuser /app

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
# openssl 3.5.7-1~deb13u2 closes CVE-2026-14456, unbounded memory growth in the
# QUIC server reachable as a denial of service. One defect, four binary packages
# -- libssl-dev, libssl3t64, openssl, openssl-provider-legacy -- so it reads as
# four HIGH findings. Gated the dev deploy on 2026-08-26, the same day and the
# same CVE that gated the frontend image against alpine.
# chromium 151.0.7922.173-1~deb13u1 closes CVE-2026-76018, arbitrary code
# execution via a crafted file in the Import component. Two HIGH findings from
# one source package (chromium, chromium-common). Gated the dev deploy on
# 2026-08-31 on a commit that touched no Docker and no dependency — the cached
# apt layer was still serving 151.0.7922.169.
# chromium 152.0.7977.75-1~deb13u1 closes CVE-2026-84349 (use after free),
# CVE-2026-84351 (buffer overflow in GPU) and CVE-2026-84357 (improper input
# validation). SIX HIGH findings, the same three CVEs counted once per source
# package (chromium, chromium-common). Gated the dev deploy on 2026-09-03 on a
# chat-pipeline commit that touched no Docker and no dependency — the cached
# apt layer was still serving 151.0.7922.173, exactly the shape of the
# 2026-08-31 entry above. The nightly trivy DB refresh is what changes; the
# image does not, until this line does.
#
# Bumping this line is the remedy, but the bump has to actually be BUILT, and a
# deploy run reports a SHA it did not necessarily build. On 2026-09-03 run
# 33816926276 reported headSha ecc8ab1f4 (this line already at 2026-09-03) while
# resolving and checking out 19bc985c4, compiling "apt security epoch:
# 2026-08-31" and failing on 151.0.7922.173. `workflow_run` deploys build the
# triggering CI run's head_sha, which lags the branch tip that GitHub stamps on
# the run — so a bump can look like it failed when it was never compiled.
#
# Before bumping this line a second time, confirm the first one was built:
# grep the deploy log for "apt security epoch:" and compare it to the ARG in the
# SHA the run claims. When those disagree, the run is not testing this tree.
# See carnet#262.
# glib 2.84.4-3~deb13u4 (CVE-2026-58010..58016, one CRITICAL), pcre2
# 10.46-1~deb13u2 (CVE-2026-86145, 89161), perl 5.40.1-6+deb13u1
# (CVE-2026-13221 and six more) and gzip 1.13-1+deb13u1 (CVE-2026-41992):
# 49 fixed HIGH/CRITICAL findings from one Debian batch gated the dev deploy
# on 2026-09-14, on a commit that touched no Docker file; the failed run
# echoes "apt security epoch: 2026-09-06".
# chromium 153.0.8010.47-2~deb13u1 (CVE-2026-87534 and 43 more, four
# CRITICAL) gated two consecutive dev deploys on 2026-09-18 — 104403e56 and
# b3ff27888, the first touching no Docker file; both runs echo "apt security
# epoch: 2026-09-14" and report 152.0.7977.82-1~deb13u1 installed.
# nodejs 20.19.2+dfsg-1+deb13u3 closes CVE-2026-48930 (CRITICAL, authority
# rebinding through embedded-NUL hostnames in TLS) and six HIGH CVEs — 21
# findings, since Trivy reports each once for nodejs, libnode115 and
# libnode-dev. Gated two consecutive dev deploys on 2026-09-27 (571b471dd and
# 3c4257b9e, neither touching a Docker file) with deb13u2 installed.
# openssl 3.5.7-1~deb13u3 closes CVE-2026-75804 (denial of service through
# unenforced QUIC limits) and CVE-2026-84782 (information disclosure): eight
# HIGH findings, each CVE once for libssl-dev, libssl3t64, openssl and
# openssl-provider-legacy. Gated two consecutive dev deploys on 2026-09-30
# (01f236f2f and 1d9673d3a, neither touching a Docker file); the 1d9673d3a run
# echoes "apt security epoch: 2026-09-27" with deb13u2 installed.
# chromium 154.0.8037.92-1~deb13u1 closes 22 HIGH findings (CVE-2026-102299
# through CVE-2026-102329: V8 type confusion, use-after-free and out-of-bounds
# reads/writes); the 2026-10-01 deploy of 9ba210b0f, which touched no Docker
# file, was gated with 154.0.8037.57-1~deb13u1 installed.
ARG APT_SECURITY_EPOCH=2026-10-01

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
# COPILOT_CLI_VERSION has a hard floor of 1.0.59: that is the first release
# whose `initialize` advertises `agentCapabilities.mcpCapabilities {http,sse}`,
# which the native MCP tool-calling bridge (COPILOT_HEADLESS_MCP_TOOL_CALLING)
# relies on. Running an older CLI (1.0.44) under native MCP + Autopilot crashed
# the `copilot --acp` subprocess mid-synthesis ("connection closed
# unexpectedly") after tool calls succeeded — the regression that started
# 2026-06-07 when the bridge was enabled while the image still pinned 1.0.44.
# The pin below tracks the published latest; `copilot-cli-version-check.yml`
# compares the two and says when they diverge.
#
# - copilot_headless LLM provider (primary): GitHub Copilot CLI in ACP mode
# - claude_code LLM provider (runtime fallback): Anthropic Claude Code CLI
#   authenticated via CLAUDE_CODE_OAUTH_TOKEN (one-year subscription token from
#   `claude setup-token`, see PIERRE_LLM_FALLBACK_PROVIDER in dev/main.tf).
#
# After install: remove npm and its transitive deps (node-tar CVEs) since npm
# is not needed at runtime.
ARG COPILOT_CLI_VERSION=1.0.86
ARG CLAUDE_CODE_CLI_VERSION=2.1.258
# The extracted copilot package vendors foundry-local-sdk, which bundles
# adm-zip 0.6.0 (CVE-2026-102282, GHSA-8238-w5pm-2374, GHSA-rcw4-f5rp-g42v;
# all fixed in 0.6.1). The trivy gate fails the image on it. adm-zip has no
# dependencies and 0.6.1 is a patch release, so every vendored copy is swapped
# for the fixed release after extraction, and the build fails if any other
# version is still there.
ARG ADM_ZIP_VERSION=0.6.1
# The copilot binary is a Node SEA bootstrap: on first run it extracts its
# package (app.js + prebuilds/linux-x64/runtime.node, ~150 MB) into
# $COPILOT_PKG_CACHE_HOME/pkg and skips the extraction whenever that directory
# already carries `.extraction-complete`. Without a shared cache the build-time
# `copilot --version` below extracts under /root while the server runs as
# appuser, so every cold instance re-extracted the package into its in-memory
# filesystem on the first `copilot --acp` spawn (150,136 KB measured; embacle
# budgets 20-25 s for it). A world-readable path lets the build fill the cache
# once for every user. COPILOT_AUTO_UPDATE=false keeps the loader from
# selecting or downloading any version other than the pinned one at runtime.
# See carnet#466.
ENV COPILOT_PKG_CACHE_HOME=/opt/copilot-pkg
ENV COPILOT_AUTO_UPDATE=false
# The copilot_sdk provider spawns this wrapper (`--server --stdio`), which loads
# the runtime.node next to it — the same pair the extraction above produced, so
# the SDK path ships nothing the ACP path did not already carry.
ENV COPILOT_RUNTIME_PATH=/opt/copilot-pkg/pkg/linux-x64/${COPILOT_CLI_VERSION}/prebuilds/linux-x64/copilot-runtime
RUN npm install -g "@github/copilot-linux-x64@${COPILOT_CLI_VERSION}" \
    && npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_CLI_VERSION}" \
    && NPM_PREFIX="$(npm prefix -g)" \
    && ln -sf "$NPM_PREFIX/bin/copilot-linux-x64" /usr/local/bin/copilot \
    && copilot --version \
    && test -f "/opt/copilot-pkg/pkg/linux-x64/${COPILOT_CLI_VERSION}/.extraction-complete" \
    && test -s "/opt/copilot-pkg/pkg/linux-x64/${COPILOT_CLI_VERSION}/prebuilds/linux-x64/runtime.node" \
    && test -x "/opt/copilot-pkg/pkg/linux-x64/${COPILOT_CLI_VERSION}/prebuilds/linux-x64/copilot-runtime" \
    && npm install --prefix /tmp/adm-zip --no-save --ignore-scripts "adm-zip@${ADM_ZIP_VERSION}" \
    && find /opt/copilot-pkg -type d -path '*/node_modules/adm-zip' -prune \
        -exec rm -rf {} \; -exec cp -R /tmp/adm-zip/node_modules/adm-zip {} \; \
    && rm -rf /tmp/adm-zip \
    && if find /opt/copilot-pkg -path '*/node_modules/adm-zip/package.json' \
        -exec grep -L "\"version\": \"${ADM_ZIP_VERSION}\"" {} + | grep -q .; then \
        echo "adm-zip other than ${ADM_ZIP_VERSION} left under /opt/copilot-pkg" >&2; exit 1; fi \
    && chmod -R a+rX /opt/copilot-pkg \
    && claude --version \
    && rm -rf "$NPM_PREFIX/lib/node_modules/npm" "$NPM_PREFIX/bin/npm" "$NPM_PREFIX/bin/npx"

# Server binary, pierre-cli (seeders + admin tools), and entrypoint scripts.
# --link builds each layer independently of the filesystem underneath, and
# --chown/--chmod set ownership and the exec bit in the copy itself rather than
# in a RUN — so no step here reads the cached layers above, and BuildKit leaves
# them in the registry.
# Ownership is numeric (appuser is uid 1001, created above); /app itself keeps
# appuser ownership through the merge.
COPY --link --chown=1001:1001 --chmod=755 --from=binaries /app/target/release-lto/pierre-mcp-server /app/pierre-mcp-server
COPY --link --chown=1001:1001 --chmod=755 --from=binaries /app/target/release-lto/pierre-cli /app/pierre-cli
# commands/ and the eval fixtures are source files, so they come from the build
# context, which the builder stage copied them from under the same .dockerignore.
COPY --link --chown=1001:1001 commands /app/commands
# Eval harness golden fixtures — admin /admin/evals/fixtures browser walks
# this directory at runtime. PIERRE_EVALS_FIXTURES_DIR (set below) points
# the runtime at this baked-in path so the route returns real data instead
# of the empty fallback when the workspace tree isn't present.
COPY --link --chown=1001:1001 crates/pierre-evals/fixtures /app/fixtures
COPY --link --chown=1001:1001 --chmod=755 docker/images/server/entrypoint.sh /app/docker-entrypoint.sh
COPY --link --chown=1001:1001 --chmod=755 docker/images/server/seed-entrypoint.sh /app/seed-entrypoint.sh
COPY --link --chmod=755 docker/images/server/chromium-headless.sh /usr/local/bin/chromium-headless

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