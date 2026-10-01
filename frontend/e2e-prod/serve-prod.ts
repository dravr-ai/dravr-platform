// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Serves the production build (dist/) with the exact response headers the nginx image sends
// ABOUTME: The headers are read from docker/images/frontend/security-headers.conf, so a CSP edit there is what the suite runs under

import { createServer, request as forward } from 'node:http';
import { connect } from 'node:net';
import { readFileSync, statSync } from 'node:fs';
import { extname, join, normalize, resolve } from 'node:path';

const HERE = import.meta.dirname;
const DIST = resolve(HERE, '..', 'dist');
const HEADERS_FILE = resolve(HERE, '..', '..', 'docker', 'images', 'frontend', 'security-headers.conf');
const PORT = Number(process.env.E2E_PROD_PORT ?? '4180');
/**
 * A Pierre server to proxy the backend prefixes to, as nginx proxies them in
 * the image (`http://127.0.0.1:8095`). Unset, they answer 502: the suite
 * mocks every one the page reads, so reaching this server is a failure.
 */
const BACKEND_URL = process.env.E2E_PROD_BACKEND_URL ? new URL(process.env.E2E_PROD_BACKEND_URL) : null;

const CONTENT_TYPES: Record<string, string> = {
  '.html': 'text/html',
  '.js': 'application/javascript',
  '.mjs': 'application/javascript',
  '.css': 'text/css',
  '.json': 'application/json',
  '.webmanifest': 'application/manifest+json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
  '.woff2': 'font/woff2',
  '.txt': 'text/plain',
};

/**
 * Every `add_header Name "value" always;` line of the nginx snippet, in
 * order. nginx sends them on every response that `include`s the snippet,
 * which in nginx.conf is every location this server stands in for.
 *
 * A snippet that yields no Content-Security-Policy is refused rather than
 * served without one: a server that dropped the policy would pass every
 * map test the policy exists to fail.
 */
function securityHeaders(): Array<[string, string]> {
  const text = readFileSync(HEADERS_FILE, 'utf8');
  const headers: Array<[string, string]> = [];
  for (const line of text.split('\n')) {
    const match = /^\s*add_header\s+([A-Za-z-]+)\s+"([^"]*)"\s+always;\s*$/.exec(line);
    if (match) headers.push([match[1], match[2]]);
  }
  if (!headers.some(([name]) => name.toLowerCase() === 'content-security-policy')) {
    throw new Error(`no Content-Security-Policy parsed from ${HEADERS_FILE}`);
  }
  return headers;
}

const HEADERS = securityHeaders();
statSync(join(DIST, 'index.html'));

/**
 * nginx.conf's three static locations: a hashed asset is served or 404s (it
 * never falls back to the shell, which is how a missing worker is caught),
 * and every other path is a file when one exists, else the SPA shell. The
 * backend prefixes nginx proxies are forwarded to {@link BACKEND_URL}, or
 * answer 502 when none is named.
 */
function resolveFile(pathname: string): string | null {
  const relative = normalize(decodeURIComponent(pathname)).replace(/^([/\\])+/, '');
  const candidate = join(DIST, relative);
  if (!candidate.startsWith(DIST)) return null;
  try {
    if (statSync(candidate).isFile()) return candidate;
  } catch {
    // Not a file under dist/.
  }
  return pathname.startsWith('/assets/') ? null : join(DIST, 'index.html');
}

const BACKEND = /^\/(api|oauth|oauth2|admin|a2a|messaging|webhooks|providers|r|fitness|\.well-known)\/|^\/(mcp|tenants|ws)(\/|$)/;

const server = createServer((request, response) => {
  // nginx.conf declares the headers at server level, so the proxy blocks inherit them too.
  for (const [name, value] of HEADERS) response.setHeader(name, value);
  const { pathname } = new URL(request.url ?? '/', 'http://localhost');
  if (BACKEND.test(pathname)) {
    if (BACKEND_URL === null) {
      response.writeHead(502, { 'Content-Type': 'text/plain' }).end('no backend behind the production build');
      return;
    }
    // nginx's proxy_pass: the backend's own answer, headers and all, streamed back.
    const upstream = forward(
      {
        host: BACKEND_URL.hostname,
        port: BACKEND_URL.port,
        method: request.method,
        path: request.url,
        headers: { ...request.headers, host: BACKEND_URL.host, 'x-forwarded-proto': 'http' },
      },
      (answer) => {
        response.writeHead(answer.statusCode ?? 502, answer.headers);
        answer.pipe(response);
      },
    );
    upstream.on('error', () => {
      if (!response.headersSent) response.writeHead(502, { 'Content-Type': 'text/plain' });
      response.end('backend unreachable');
    });
    request.pipe(upstream);
    return;
  }
  const file = resolveFile(pathname);
  if (file === null) {
    response.writeHead(404, { 'Content-Type': 'text/plain' }).end('not found');
    return;
  }
  response.writeHead(200, {
    'Content-Type': CONTENT_TYPES[extname(file)] ?? 'application/octet-stream',
    'Cache-Control': 'no-cache',
  });
  response.end(readFileSync(file));
});

// A WebSocket upgrade on a backend prefix is spliced through to the backend unchanged.
server.on('upgrade', (request, socket, head) => {
  const { pathname } = new URL(request.url ?? '/', 'http://localhost');
  if (BACKEND_URL === null || !BACKEND.test(pathname)) {
    socket.destroy();
    return;
  }
  const upstream = connect(Number(BACKEND_URL.port), BACKEND_URL.hostname, () => {
    const lines = [`${request.method} ${request.url} HTTP/1.1`];
    for (let i = 0; i < request.rawHeaders.length; i += 2) {
      lines.push(`${request.rawHeaders[i]}: ${request.rawHeaders[i + 1]}`);
    }
    upstream.write(`${lines.join('\r\n')}\r\n\r\n`);
    upstream.write(head);
    upstream.pipe(socket);
    socket.pipe(upstream);
  });
  upstream.on('error', () => socket.destroy());
  socket.on('error', () => upstream.destroy());
});

server.listen(PORT, '127.0.0.1', () => {
  const backend = BACKEND_URL === null ? 'no backend' : `backend ${BACKEND_URL.origin}`;
  console.log(`production build on http://127.0.0.1:${PORT} with ${HEADERS.length} headers from security-headers.conf, ${backend}`);
});
