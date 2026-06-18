// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Server lifecycle management helper for integration tests
// ABOUTME: Starts, monitors health, and cleanly shuts down Pierre MCP server instances
//
// Licensed under either of Apache License, Version 2.0 or MIT License at your option.
// Copyright (c) 2026 dravr.ai

const { spawn } = require('child_process');
const path = require('path');
const { TestConfig } = require('./fixtures');

// Use native fetch (Node 18+) or dynamic import for node-fetch
const fetch = global.fetch || (async (...args) => {
  const nodeFetch = await import('node-fetch');
  return nodeFetch.default(...args);
});

/**
 * Ensures Pierre MCP server is running
 * - In CI: Always starts fresh server
 * - Locally: Uses existing server if available
 */
async function ensureServerRunning(config = {}) {
  const port = config.port || process.env.PIERRE_SERVER_PORT || process.env.HTTP_PORT || process.env.MCP_PORT || 8081;
  const healthUrl = `http://localhost:${port}/health`;

  // Reuse an already-running server. The integration/e2e runs start ONE shared
  // server in jest globalSetup; locally a dev may already have one up. Starting a
  // fresh server per suite races server lifecycles on the shared port (bind
  // failures + cold-start health-check timeouts) — see test/helpers/global-setup.js.
  try {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 2000);
    const response = await fetch(healthUrl, { signal: controller.signal });
    clearTimeout(timeout);

    if (response.ok) {
      console.log('✅ Using running Pierre server');
      const databaseUrl = config.database || TestConfig.testDatabase;
      // The encryption key MUST match the running server's; otherwise pierre-cli
      // refuses to open the DB ("Encryption key mismatch") and promotion fails.
      const encryptionKey = config.encryptionKey || TestConfig.testEncryptionKey;
      const testToken = await registerAndGetToken(port, databaseUrl, encryptionKey);
      return { process: null, port, logs: [], testToken, cleanup: null };
    }
  } catch (error) {
    // Server not running
  }

  console.log('🚀 Starting Pierre server for tests...');
  return await startServer({ port, ...config });
}

/**
 * Start Pierre MCP server for testing
 */
async function startServer(config) {
  const port = config.port || process.env.HTTP_PORT || process.env.MCP_PORT || 8081;

  // Try multiple possible locations for the server binary
  // Supports: explicit config, relative paths from sdk/test/helpers/, and PIERRE_SERVER_BINARY env var
  const possiblePaths = [
    config.binaryPath,
    process.env.PIERRE_SERVER_BINARY,
    path.join(__dirname, '../../../target/debug/pierre-mcp-server'),
    path.join(__dirname, '../../../target/release/pierre-mcp-server')
  ].filter(Boolean);

  let serverPath = null;
  const fs = require('fs');
  for (const testPath of possiblePaths) {
    if (fs.existsSync(testPath)) {
      serverPath = testPath;
      break;
    }
  }

  if (!serverPath) {
    throw new Error('Pierre server binary not found. Please run: cargo build --bin pierre-mcp-server');
  }

  // Default to the shared file-backed test DB (NOT in-memory): the global-admin
  // promotion runs pierre-cli as a SEPARATE process, which can only reach a
  // file-backed DB. An in-memory default silently skips promotion → the test
  // user stays non-admin → tools/list returns the tenant-filtered subset.
  const databaseUrl = config.database || TestConfig.testDatabase;
  const encryptionKey = config.encryptionKey || 'rEFe91l6lqLahoyl9OSzum9dKa40VvV5RYj8bHGNTeo=';
  const env = {
    ...process.env,
    HTTP_PORT: port.toString(),
    DATABASE_URL: databaseUrl,
    // Integration tests register users over HTTP, which default to "Pending" (admin
    // approval). Auto-approve so the issued token belongs to an Active user and
    // authenticates against /mcp (which rejects non-Active users with 401, RFC 9728).
    AUTO_APPROVE_USERS: 'true',
    PIERRE_MASTER_ENCRYPTION_KEY: encryptionKey,
    PIERRE_JWT_SECRET: config.jwtSecret || 'test_jwt_secret_for_automated_tests_only',
    PIERRE_RSA_KEY_SIZE: '2048', // Use smaller key size for faster test startup
    RUST_LOG: config.logLevel || 'info'
  };

  // CRITICAL: UNSET the Resend config so the auto-approval "account approved" email
  // is never actually sent. AUTO_APPROVE_USERS above approves each registered
  // sdk-test-*@example.com user, which would otherwise fire a real Resend email
  // (bouncing/delaying and flooding the Resend dashboard). The server builds its
  // email service only when RESEND_API_KEY is set, and `env::var().ok()` treats an
  // empty string as Some("") — so we must DELETE the inherited vars, not blank them.
  delete env.RESEND_API_KEY;
  delete env.RESEND_FROM_EMAIL;

  const serverProcess = spawn(serverPath, [], {
    env,
    stdio: process.env.DEBUG ? 'inherit' : 'pipe',
    detached: false
  });

  const logs = [];
  if (serverProcess.stdout) {
    serverProcess.stdout.on('data', (data) => {
      logs.push(data.toString());
      if (process.env.DEBUG) {
        console.log(`[Server]: ${data}`);
      }
    });
  }

  if (serverProcess.stderr) {
    serverProcess.stderr.on('data', (data) => {
      logs.push(data.toString());
      if (process.env.DEBUG) {
        console.error(`[Server Error]: ${data}`);
      }
    });
  }

  serverProcess.on('error', (error) => {
    console.error(`❌ Failed to start server: ${error.message}`);
  });

  try {
    // Generous timeout: globalSetup starts the one shared server for the whole
    // run, so a slow cold boot here would otherwise fail every suite at once.
    await waitForHealth(`http://localhost:${port}/health`, 90000);
  } catch (error) {
    serverProcess.kill('SIGKILL');
    console.error('Server logs:', logs.join('\n'));
    throw error;
  }

  // Register a test user and get a real RS256 JWT token for authenticated tests
  const testToken = await registerAndGetToken(port, databaseUrl, encryptionKey);

  return {
    process: serverProcess,
    port,
    logs,
    testToken,
    cleanup: async () => {
      return new Promise((resolve) => {
        serverProcess.on('exit', resolve);
        serverProcess.kill('SIGTERM');
        setTimeout(() => {
          if (!serverProcess.killed) {
            serverProcess.kill('SIGKILL');
          }
          resolve();
        }, 5000);
      });
    }
  };
}

/**
 * Promote an already-registered user to a GLOBAL admin via pierre-cli.
 *
 * tools/list returns the complete tool catalog only for a global admin
 * (`User.is_admin`); a plain tenant owner is correctly limited to their plan's
 * tenant-filtered subset (e.g. starter-tier tenants don't see professional-gated
 * analytics tools). Catalog-completeness assertions therefore require a global
 * admin. The CLI writes directly to the shared file-backed SQLite DB the server
 * reads, and the server resolves is_admin on every request, so the next login's
 * token authenticates as admin without a server restart.
 *
 * No-op for in-memory databases, which are private to the server process and
 * unreachable from a separate CLI invocation.
 */
async function promoteUserToGlobalAdmin(email, password, databaseUrl, encryptionKey) {
  if (!databaseUrl || databaseUrl.includes(':memory:')) {
    console.warn(
      `⚠️ DATABASE_URL is in-memory (${databaseUrl}); cannot promote ${email} to global admin from CLI`
    );
    return false;
  }

  const fs = require('fs');
  const cliCandidates = [
    process.env.PIERRE_CLI_BINARY,
    path.join(__dirname, '../../../target/debug/pierre-cli'),
    path.join(__dirname, '../../../target/release/pierre-cli')
  ].filter(Boolean);
  const cliPath = cliCandidates.find((p) => fs.existsSync(p));
  if (!cliPath) {
    throw new Error('pierre-cli binary not found. Please run: cargo build --bin pierre-cli');
  }

  return new Promise((resolve, reject) => {
    const proc = spawn(
      cliPath,
      [
        'user',
        'create',
        '--email',
        email,
        '--password',
        password,
        '--force',
        '--super-admin'
      ],
      {
        env: {
          ...process.env,
          DATABASE_URL: databaseUrl,
          PIERRE_MASTER_ENCRYPTION_KEY:
            encryptionKey || process.env.PIERRE_MASTER_ENCRYPTION_KEY,
          RUST_LOG: 'warn'
        },
        stdio: 'pipe'
      }
    );

    let stderr = '';
    proc.stderr.on('data', (d) => {
      stderr += d.toString();
    });
    proc.on('error', reject);
    proc.on('close', (code) => {
      if (code === 0) {
        console.log(`🔑 Promoted ${email} to global admin (full tool catalog)`);
        resolve(true);
      } else {
        reject(new Error(`pierre-cli promotion failed (code ${code}):\n${stderr}`));
      }
    });
  });
}

/**
 * Register a test user, promote it to a global admin, and login to get a real
 * RS256 JWT token. The server uses RS256 (RSA) JWT validation, so test tokens
 * must come from the actual server login endpoint rather than being locally
 * generated. The global-admin promotion makes tools/list return the full tool
 * catalog (see promoteUserToGlobalAdmin).
 */
async function registerAndGetToken(port, databaseUrl, encryptionKey) {
  const baseUrl = `http://localhost:${port}`;
  const testEmail = `sdk-test-${Date.now()}@example.com`;
  const testPassword = 'SdkTestPassword123!';

  // Register user (may be auto-approved or pending depending on config)
  try {
    const registerResponse = await fetch(`${baseUrl}/api/auth/register`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        email: testEmail,
        password: testPassword,
        display_name: 'SDK Test User'
      })
    });

    if (registerResponse.ok) {
      const registered = await registerResponse.json();
      console.log(`📋 Registered ${testEmail} (user_status=${registered.user_status})`);
    } else {
      const errorText = await registerResponse.text();
      console.warn(`⚠️ Registration returned ${registerResponse.status}: ${errorText}`);
    }
  } catch (error) {
    console.warn(`⚠️ Registration failed: ${error.message}`);
  }

  // Promote to a global admin so tools/list returns the full catalog.
  await promoteUserToGlobalAdmin(testEmail, testPassword, databaseUrl, encryptionKey);

  // Login via OAuth2 ROPC (RFC 6749 §4.3) to get RS256 JWT
  try {
    const loginResponse = await fetch(`${baseUrl}/oauth/token`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        grant_type: 'password',
        username: testEmail,
        password: testPassword
      }).toString()
    });

    if (loginResponse.ok) {
      const tokenData = await loginResponse.json();
      console.log('✅ Test user authenticated with RS256 JWT');
      return {
        access_token: tokenData.access_token,
        token_type: tokenData.token_type || 'Bearer',
        expires_in: tokenData.expires_in || 86400,
        scope: tokenData.scope || 'read:fitness write:fitness',
        saved_at: Math.floor(Date.now() / 1000)
      };
    }

    const errorText = await loginResponse.text();
    console.warn(`⚠️ Login returned ${loginResponse.status}: ${errorText}`);
  } catch (error) {
    console.warn(`⚠️ Login failed: ${error.message}`);
  }

  // Fallback: return a placeholder token (tests that require auth will fail with clear errors)
  console.warn('⚠️ Could not get RS256 token - tests requiring auth will fail');
  return {
    access_token: 'INVALID_NO_RS256_TOKEN_AVAILABLE',
    token_type: 'Bearer',
    expires_in: 3600,
    scope: 'read:fitness write:fitness',
    saved_at: Math.floor(Date.now() / 1000)
  };
}

/**
 * Wait for server health check to pass
 */
async function waitForHealth(url, timeout = 30000) {
  const startTime = Date.now();

  while (Date.now() - startTime < timeout) {
    try {
      const controller = new AbortController();
      const fetchTimeout = setTimeout(() => controller.abort(), 1000);
      const response = await fetch(url, { signal: controller.signal });
      clearTimeout(fetchTimeout);

      if (response.ok) {
        console.log('✅ Server health check passed');
        return;
      }
    } catch (error) {
      // Not ready yet
    }
    await sleep(500);
  }

  throw new Error(`Server health check failed after ${timeout}ms`);
}

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

module.exports = {
  ensureServerRunning,
  startServer,
  registerAndGetToken,
  promoteUserToGlobalAdmin,
  waitForHealth,
  sleep
};
