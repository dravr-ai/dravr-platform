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
};
