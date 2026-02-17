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

RUN cd frontend && bun run build

# ---------------------------------------------------------------------------
# Runtime stage: serve static files with nginx
# ---------------------------------------------------------------------------
FROM nginx:1.27-alpine

# Copy built assets from the builder stage
COPY --from=builder /app/frontend/dist /usr/share/nginx/html

# Copy custom nginx configuration
COPY docker/images/frontend/nginx.conf /etc/nginx/nginx.conf

EXPOSE 8080

CMD ["nginx", "-g", "daemon off;"]
