// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Loopback HTTP helpers for the OAuth callback server unit tests
// ABOUTME: Picks free ports, drives raw requests, and starts/stops a bound callback listener

const http = require('http');
const net = require('net');
const { PierreOAuthClientProvider } = require('../../dist/index.js');

const SERVER_URL = 'http://localhost:8081';

/** Reserve then release a port so a test can hand a known-free port to the client. */
function freePort() {
  return new Promise((resolve, reject) => {
    const probe = net.createServer();
    probe.once('error', reject);
    probe.listen(0, 'localhost', () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
}

/** Hold a port open, standing in for another local process that got there first. */
function occupyPort(port) {
  return new Promise((resolve, reject) => {
    const squatter = net.createServer();
    squatter.once('error', reject);
    squatter.listen(port ?? 0, 'localhost', () => resolve(squatter));
  });
}

/**
 * Resolves once the port can be bound again, rejecting if it is still held. Retries
 * briefly because closing a listener is asynchronous; a port that is never released
 * exhausts every attempt and fails.
 */
async function portIsFree(port, attempts = 20) {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return await new Promise((resolve, reject) => {
        const probe = net.createServer();
        probe.once('error', reject);
        probe.listen(port, 'localhost', () => probe.close(() => resolve(port)));
      });
    } catch (error) {
      if (attempt >= attempts) {
        throw error;
      }
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  }
}

/**
 * Raw loopback request. Uses the 'localhost' name rather than a literal address so the
 * request reaches the callback server whichever loopback family it resolved to.
 */
function httpRequest(port, { method = 'GET', path = '/', headers = {}, json, body } = {}) {
  const payload = json !== undefined ? JSON.stringify(json) : body;
  const allHeaders = { ...headers };
  if (payload !== undefined) {
    allHeaders['Content-Type'] = allHeaders['Content-Type'] || 'application/json';
    allHeaders['Content-Length'] = Buffer.byteLength(payload);
  }
  return new Promise((resolve, reject) => {
    const req = http.request({ host: 'localhost', port, method, path, headers: allHeaders }, (res) => {
      let data = '';
      res.on('data', (chunk) => {
        data += chunk;
      });
      res.on('end', () => resolve({ status: res.statusCode, body: data }));
    });
    req.on('error', reject);
    if (payload !== undefined) {
      req.write(payload);
    }
    req.end();
  });
}

function makeConfig(overrides = {}) {
  return {
    mode: 'oauth',
    pierreServerUrl: SERVER_URL,
    oauthClientId: 'test-client',
    oauthClientSecret: 'test-secret',
    ...overrides,
  };
}

/** Silent provider so a test run is not buried in the client's flow logging. */
function makeProvider(config = {}, serverUrl = SERVER_URL) {
  const provider = new PierreOAuthClientProvider(serverUrl, makeConfig(config));
  provider.log = () => {};
  return provider;
}

/** Provider whose callback listener is bound and ready to take requests. */
async function startProvider(config = {}, serverUrl = SERVER_URL) {
  const port = config.callbackPort ?? (await freePort());
  const provider = makeProvider({ ...config, callbackPort: port }, serverUrl);
  const boundPort = await provider.ensureCallbackServerBound();
  return { provider, port, boundPort };
}

function stopProvider(provider) {
  if (provider) {
    provider.teardownAuthorizationFlow();
  }
}

function closeServer(server) {
  if (!server) {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    server.closeAllConnections?.();
    server.close(() => resolve());
  });
}

module.exports = {
  SERVER_URL,
  closeServer,
  freePort,
  httpRequest,
  makeConfig,
  makeProvider,
  occupyPort,
  portIsFree,
  startProvider,
  stopProvider,
};
