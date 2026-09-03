// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One scripted 2026-07-28 endpoint for the unit suites — JSON, SSE, held replies, tasks
// ABOUTME: Replaces two hand-rolled copies of the same http.createServer scaffold

/**
 * A fake Dravr, once.
 *
 * The client suite and the bridge suite each grew their own `http.createServer`
 * with the same accumulate-body / record-request / reply shape, and their own
 * copy of the `complete()` envelope builder. Two fakes of one protocol drift:
 * a reply shape fixed in one suite stays wrong in the other, and the bug hides
 * in whichever test did not get the fix.
 */

const http = require('http');

/** The `result` envelope a 2026-07-28 server returns for an inline answer. */
const complete = (id, result) => ({
  jsonrpc: '2.0',
  id,
  result: { resultType: 'complete', ...result },
});

/**
 * A scripted MCP endpoint.
 *
 * `handle(rpc, req)` receives the parsed JSON-RPC request and the raw HTTP
 * request, and returns what to send:
 *
 * - `{ status, headers, json }` — one JSON object
 * - `{ sse: [message, ...] }` — an event stream carrying those messages
 * - `{ hold: true }` — never answered, so a caller's timeout or abort is what
 *   ends the request
 *
 * The resolved handle carries `seen` (every request, with headers), `url` (the
 * `/mcp` endpoint), `origin` (the server root, for a bridge that appends its
 * own path) and `close()`.
 */
function startEndpoint(handle) {
  const seen = [];
  const held = [];
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      let body = '';
      req.on('data', (chunk) => {
        body += chunk;
      });
      req.on('end', () => {
        const rpc = JSON.parse(body);
        seen.push({ headers: req.headers, rpc, url: req.url });
        const reply = handle(rpc, req) || {};
        if (reply.hold) {
          held.push(res);
          return;
        }
        if (reply.sse) {
          res.writeHead(200, { 'Content-Type': 'text/event-stream' });
          for (const message of reply.sse) {
            res.write(`data: ${JSON.stringify(message)}\n\n`);
          }
          res.end();
          return;
        }
        const status = reply.status ?? 200;
        res.writeHead(status, {
          'Content-Type': 'application/json',
          ...(reply.headers || {}),
        });
        res.end(reply.json === undefined ? '' : JSON.stringify(reply.json));
      });
    });
    server.listen(0, '127.0.0.1', () => {
      const origin = `http://127.0.0.1:${server.address().port}`;
      resolve({
        server,
        seen,
        origin,
        url: `${origin}/mcp`,
        close: () =>
          new Promise((done) => {
            for (const res of held) {
              res.destroy();
            }
            server.closeAllConnections();
            server.close(done);
          }),
      });
    });
  });
}

module.exports = { complete, startEndpoint };
