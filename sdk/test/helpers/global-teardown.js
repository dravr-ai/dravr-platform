// ABOUTME: Jest globalTeardown — stops the shared Pierre server started in globalSetup.
// ABOUTME: Reads the persisted PID and sends SIGTERM so no server process leaks.

const fs = require('fs');
const path = require('path');
const os = require('os');
const { TestConfig } = require('./fixtures');

const PID_FILE = path.join(os.tmpdir(), 'pierre-test-server.pid');

module.exports = async () => {
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  try {
    const pid = parseInt(fs.readFileSync(PID_FILE, 'utf8'), 10);
    process.kill(pid, 'SIGTERM');
    // Wait for the server to FULLY exit before unlinking the DB below. Removing the
    // SQLite file out from under a still-running server leaves the next test step's
    // server with a half-initialized DB whose register/login fail (500 DatabaseError
    // -> 400 invalid_grant). Poll the pid until it is gone (process.kill throws
    // ESRCH), then SIGKILL as a backstop if it outlives a graceful shutdown.
    let alive = true;
    for (let i = 0; i < 150 && alive; i++) {
      try {
        process.kill(pid, 0);
        await sleep(100);
      } catch (error) {
        alive = false;
      }
    }
    if (alive) {
      try {
        process.kill(pid, 'SIGKILL');
        await sleep(500);
      } catch (error) {
        // Already gone between the poll and the SIGKILL.
      }
    }
    fs.unlinkSync(PID_FILE);
    console.log(`🧹 Shared test server (pid ${pid}) stopped`);
  } catch (error) {
    // Server already gone or PID file missing — nothing to clean up.
  }

  // Remove the file-backed test DB (and its WAL/SHM sidecars) so it doesn't
  // leak admin/catalog state into the next run.
  for (const suffix of ['', '-wal', '-shm', '-journal']) {
    try {
      fs.unlinkSync(TestConfig.testDatabaseFile + suffix);
    } catch (error) {
      // Already gone — nothing to clean up.
    }
  }
};
