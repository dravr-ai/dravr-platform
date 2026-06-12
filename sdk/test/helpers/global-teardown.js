// ABOUTME: Jest globalTeardown — stops the shared Pierre server started in globalSetup.
// ABOUTME: Reads the persisted PID and sends SIGTERM so no server process leaks.

const fs = require('fs');
const path = require('path');
const os = require('os');

const PID_FILE = path.join(os.tmpdir(), 'pierre-test-server.pid');

module.exports = async () => {
  try {
    const pid = parseInt(fs.readFileSync(PID_FILE, 'utf8'), 10);
    process.kill(pid, 'SIGTERM');
    fs.unlinkSync(PID_FILE);
    console.log(`🧹 Shared test server (pid ${pid}) stopped`);
  } catch (error) {
    // Server already gone or PID file missing — nothing to clean up.
  }
};
