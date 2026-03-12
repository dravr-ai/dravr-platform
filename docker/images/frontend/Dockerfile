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

ARG VITE_API_BASE_URL
ENV VITE_API_BASE_URL=${VITE_API_BASE_URL}

# Firebase configuration (public keys, baked at build time)
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
FROM nginx:1.28-alpine3.23

# Upgrade system packages to patch CVEs:
# - zlib >= 1.3.2-r0 (CVE-2026-22184 critical, CVE-2026-27171 medium)
# - libpng >= 1.6.55-r0 (CVE-2026-25646 high)
RUN apk upgrade --no-cache && \
    apk add --no-cache zlib>=1.3.2-r0 libpng>=1.6.55-r0

# Copy built assets from the builder stage
COPY --from=builder /app/frontend/dist /usr/share/nginx/html

# Copy custom nginx configuration
COPY docker/images/frontend/nginx.conf /etc/nginx/nginx.conf

EXPOSE 8080

CMD ["nginx", "-g", "daemon off;"]
