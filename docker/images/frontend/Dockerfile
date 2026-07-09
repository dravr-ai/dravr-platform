# ABOUTME: Multi-stage build for Pierre admin frontend served by nginx
# ABOUTME: Builds Vite SPA with bun, serves static files via nginx-alpine

# ---------------------------------------------------------------------------
# Build stage: install dependencies and build the Vite SPA
# ---------------------------------------------------------------------------
FROM oven/bun:1.3 AS builder

WORKDIR /app

COPY package.json bun.lock ./

COPY packages/ packages/
COPY frontend/package.json frontend/package.json
COPY frontend-mobile/package.json frontend-mobile/package.json

RUN bun install --frozen-lockfile --filter frontend

COPY frontend/ frontend/

# Cache-buster: changes when the commit SHA changes, forcing a rebuild
ARG COMMIT_SHA=dev
ENV COMMIT_SHA=${COMMIT_SHA}

# Firebase configuration (public keys, baked at build time)
# VITE_API_BASE_URL is intentionally omitted — frontend uses same-origin (nginx proxy)
ARG VITE_FIREBASE_API_KEY
ARG VITE_FIREBASE_AUTH_DOMAIN
ARG VITE_FIREBASE_PROJECT_ID
ARG VITE_FIREBASE_STORAGE_BUCKET
ARG VITE_FIREBASE_MESSAGING_SENDER_ID
ARG VITE_FIREBASE_APP_ID
ENV VITE_FIREBASE_API_KEY=${VITE_FIREBASE_API_KEY}
ENV VITE_FIREBASE_AUTH_DOMAIN=${VITE_FIREBASE_AUTH_DOMAIN}
ENV VITE_FIREBASE_PROJECT_ID=${VITE_FIREBASE_PROJECT_ID}
ENV VITE_FIREBASE_STORAGE_BUCKET=${VITE_FIREBASE_STORAGE_BUCKET}
ENV VITE_FIREBASE_MESSAGING_SENDER_ID=${VITE_FIREBASE_MESSAGING_SENDER_ID}
ENV VITE_FIREBASE_APP_ID=${VITE_FIREBASE_APP_ID}

RUN cd frontend && bun run build

# ---------------------------------------------------------------------------
# Runtime stage: serve static files with nginx
# ---------------------------------------------------------------------------
# Use the plain Alpine base + Alpine's own nginx package, NOT the official
# nginx:alpine image. The official image installs nginx from nginx.org's apk repo,
# which still ships 1.28.3-r1 (CVE-2026-9256, ngx_http_rewrite_module RCE/DoS);
# `apk upgrade` cannot move it because alpine main is a different repo source.
# Alpine 3.23 main ships the patched nginx (>= 1.28.3-r2), so installing from
# there fixes the CVE at the source instead of allowlisting it.
FROM alpine:3.23

# Cache-bust token for the apk layer below. buildx reuses the `apk upgrade` layer
# verbatim across builds even after Alpine ships a security update, leaving patched
# base packages (e.g. openssl libssl3/libcrypto3) stale. Bump this date whenever
# Trivy flags a fixed Alpine CVE to force a fresh `apk upgrade` that pulls the
# patched version (e.g. CVE-2026-45447 in libssl3/libcrypto3).
ARG APK_SECURITY_EPOCH=2026-06-10

# nginx (patched, fixes CVE-2026-9256) + gettext for envsubst (the official nginx
# image bundled envsubst; plain alpine does not). apk upgrade patches the rest of
# the base (zlib/libpng/etc.) to current 3.23 versions.
RUN echo "apk security epoch: ${APK_SECURITY_EPOCH}" && \
    apk upgrade --no-cache && \
    apk add --no-cache nginx gettext && \
    rm -rf /var/cache/apk/* && \
    # Stream nginx logs to the container stdout/stderr (Cloud Run captures them) —
    # mirrors what the official nginx image did via symlinks.
    mkdir -p /var/log/nginx && \
    ln -sf /dev/stdout /var/log/nginx/access.log && \
    ln -sf /dev/stderr /var/log/nginx/error.log

# Copy built assets from the builder stage
COPY --from=builder /app/frontend/dist /usr/share/nginx/html

# Copy nginx config template (envsubst injects BACKEND_URL at startup)
COPY docker/images/frontend/nginx.conf /etc/nginx/nginx.conf.template

# Shared security-header snippet `include`d per-location by the config above.
# No env vars, so it is copied verbatim (not run through envsubst).
COPY docker/images/frontend/security-headers.conf /etc/nginx/security-headers.conf

EXPOSE 8080

# Substitute BACKEND_URL and FIREBASE_PROJECT_ID into nginx config at runtime, then start nginx
CMD ["/bin/sh", "-c", "envsubst '${BACKEND_URL} ${FIREBASE_PROJECT_ID}' < /etc/nginx/nginx.conf.template > /etc/nginx/nginx.conf && nginx -g 'daemon off;'"]
