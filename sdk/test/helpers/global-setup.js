// ABOUTME: Jest globalSetup — starts ONE shared Pierre server for the whole test run.
// ABOUTME: Avoids per-suite server churn that races server lifecycles on a shared port.

const { startServer } = require('./server');
const { TestConfig } = require('./fixtures');
const fs = require('fs');
const path = require('path');
const os = require('os');

// PID is persisted to a temp file so globalTeardown (a separate module
// invocation) can stop the same server process.
const PID_FILE = path.join(os.tmpdir(), 'pierre-test-server.pid');

module.exports = async () => {
  // Remove any stale file-backed test DB (and its WAL/SHM sidecars) from a prior
  // run so each run starts hermetic — the catalog/admin state must not leak
  // across runs. The DB is file-backed (not in-memory) so pierre-cli can promote
  // the registered test user to a global admin in the same DB the server reads.
  for (const suffix of ['', '-wal', '-shm', '-journal']) {
    try {
      fs.unlinkSync(TestConfig.testDatabaseFile + suffix);
    } catch (error) {
      // Missing file is the normal case on a clean run.
    }
  }

  // Starting one server up front means every suite's ensureServerRunning() finds
  // it healthy and reuses it, instead of spawning/killing a fresh server per
  // describe block — the source of port-reuse bind failures and cold-start
  // health-check timeouts when many suites run sequentially on the same port.
  const handle = await startServer({
    port: TestConfig.defaultServerPort,
    database: TestConfig.testDatabase,
    encryptionKey: TestConfig.testEncryptionKey,
  });

  fs.writeFileSync(PID_FILE, String(handle.process.pid));
  console.log(
    `✅ Shared test server ready (pid ${handle.process.pid}, port ${TestConfig.defaultServerPort})`
  );

  // Self-check: confirm the shared server's issued token actually authenticates
  // /mcp. Surfaces auth misconfiguration (e.g. unapproved user -> 401) up front
  // instead of as 20 opaque per-test failures.
  const tok = handle.testToken && handle.testToken.access_token;
  const res = await fetch(`http://localhost:${TestConfig.defaultServerPort}/mcp`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${tok}` },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list', params: {} }),
  });
  const body = await res.json().catch(() => ({}));
  console.log(
    `🔎 globalSetup /mcp self-check: status=${res.status} tools=${
      body.result && body.result.tools ? body.result.tools.length : 'n/a'
    } error=${JSON.stringify(body.error || null)}`
  );
};
