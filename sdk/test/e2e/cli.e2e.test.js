// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E tests for CLI commands in the SDK.
// ABOUTME: Tests all CLI operations including help, version, and main functionality.

const { spawn, execSync } = require('child_process');
const path = require('path');

const TIMEOUT = 30000;
const CLI_PATH = path.join(__dirname, '../../dist/cli.js');

// Credentials reach the client through the environment, never through argv: process
// arguments are world-readable (`ps -ef`, /proc/<pid>/cmdline). `options.token` supplies
// the JWT and `options.env` any other variable a case needs.
const execCli = (args = [], options = {}) => {
  return new Promise((resolve, reject) => {
    const proc = spawn('node', [CLI_PATH, ...args], {
      env: {
        ...process.env,
        PIERRE_SERVER_URL: options.serverUrl || 'http://localhost:8081',
        PIERRE_JWT_TOKEN: options.token || '',
        CI: 'true',
        ...options.env,
      },
      timeout: options.timeout || 10000,
    });

    let stdout = '';
    let stderr = '';

    proc.stdout.on('data', (data) => {
      stdout += data.toString();
    });

    proc.stderr.on('data', (data) => {
      stderr += data.toString();
    });

    proc.on('close', (code) => {
      resolve({ code, stdout, stderr });
    });

    proc.on('error', reject);

    // Auto-close after timeout for long-running processes
    if (options.autoClose) {
      setTimeout(() => {
        proc.kill('SIGTERM');
      }, options.autoClose);
    }
  });
};

describe('CLI E2E Tests', () => {
  describe('Help and Version', () => {
    test('should display help with --help flag', async () => {
      const result = await execCli(['--help']);

      expect(result.code).toBe(0);
      expect(result.stdout).toContain('pierre-mcp-client');
      expect(result.stdout).toContain('MCP client');
      expect(result.stdout).toContain('--server');
      expect(result.stdout).toContain('PIERRE_JWT_TOKEN');
    }, TIMEOUT);

    test('should display version with --version flag', async () => {
      const result = await execCli(['--version']);

      expect(result.code).toBe(0);
      // Version should be a semver-like string
      expect(result.stdout).toMatch(/\d+\.\d+\.\d+/);
    }, TIMEOUT);

    test('should display help with -h flag', async () => {
      const result = await execCli(['-h']);

      expect(result.code).toBe(0);
      expect(result.stdout).toContain('pierre-mcp-client');
    }, TIMEOUT);
  });

  describe('Server URL Option', () => {
    test('should accept --server option', async () => {
      const result = await execCli(['--server', 'http://localhost:9999', '--help']);

      expect(result.code).toBe(0);
    }, TIMEOUT);

    test('should accept -s shorthand for server', async () => {
      const result = await execCli(['-s', 'http://localhost:9999', '--help']);

      expect(result.code).toBe(0);
    }, TIMEOUT);
  });

  describe('Token Credential', () => {
    test('should select JWT auth mode from PIERRE_JWT_TOKEN', async () => {
      const result = await execCli(['--verbose'], {
        token: 'test-jwt-token',
        autoClose: 3000,
      });

      expect(result.stderr).toContain('auth mode = jwt');
      // The credential itself never reaches a log the host displays
      expect(result.stderr).not.toContain('test-jwt-token');
    }, TIMEOUT);

    test('should reject --token', async () => {
      const result = await execCli(['--token', 'test-jwt-token']);

      expect(result.code).toBe(1);
      expect(result.stderr).toContain("unknown option '--token'");
    }, TIMEOUT);

    test('should reject -t', async () => {
      const result = await execCli(['-t', 'test-jwt-token']);

      expect(result.code).toBe(1);
      expect(result.stderr).toContain("unknown option '-t'");
    }, TIMEOUT);
  });

  describe('OAuth Options', () => {
    test('should accept --oauth-client-id option', async () => {
      const result = await execCli(['--oauth-client-id', 'test-client-id', '--help']);

      expect(result.code).toBe(0);
    }, TIMEOUT);

    test('should pair --oauth-client-id with PIERRE_OAUTH_CLIENT_SECRET', async () => {
      const result = await execCli(['--verbose', '--oauth-client-id', 'test-client-id'], {
        env: { PIERRE_OAUTH_CLIENT_SECRET: 'test-secret' },
        autoClose: 3000,
      });

      expect(result.stderr).toContain('auth mode = oauth');
      expect(result.stderr).toContain('OAuth client = test-client-id');
      expect(result.stderr).not.toContain('test-secret');
    }, TIMEOUT);

    test('should accept --callback-port option', async () => {
      const result = await execCli(['--callback-port', '35536', '--help']);

      expect(result.code).toBe(0);
    }, TIMEOUT);

    test('should accept --no-browser option', async () => {
      const result = await execCli(['--no-browser', '--help']);

      expect(result.code).toBe(0);
    }, TIMEOUT);
  });

  describe('Timeout Options', () => {
    test('should accept --token-validation-timeout option', async () => {
      const result = await execCli(['--token-validation-timeout', '5000', '--help']);

      expect(result.code).toBe(0);
    }, TIMEOUT);

    test('should accept --proactive-connection-timeout option', async () => {
      const result = await execCli(['--proactive-connection-timeout', '10000', '--help']);

      expect(result.code).toBe(0);
    }, TIMEOUT);

    test('should accept --tool-call-connection-timeout option', async () => {
      const result = await execCli(['--tool-call-connection-timeout', '15000', '--help']);

      expect(result.code).toBe(0);
    }, TIMEOUT);
  });

  describe('Environment Variables', () => {
    test('should read PIERRE_SERVER_URL from environment', async () => {
      const result = await execCli(['--verbose'], {
        serverUrl: 'http://env-server:8081',
        autoClose: 3000,
      });

      // Should start (or fail to connect) but not error on parsing
      expect(result.stderr).toContain('PIERRE_SERVER_URL');
    }, TIMEOUT);

    test('should read PIERRE_JWT_TOKEN from environment', async () => {
      const result = await execCli(['--verbose'], {
        token: 'env-jwt-token',
        autoClose: 3000,
      });

      // Diagnostics report presence, never the credential itself
      expect(result.stderr).toContain('PIERRE_JWT_TOKEN');
      expect(result.stderr).toContain('[SET]');
    }, TIMEOUT);
  });

  describe('Bridge Startup', () => {
    test('should output startup diagnostics under --verbose', async () => {
      const result = await execCli(['--verbose'], { autoClose: 3000 });

      expect(result.stderr).toContain('pierre-mcp-client');
      expect(result.stderr).toContain('starting');
    }, TIMEOUT);

    test('a plain launch keeps the host log clean', async () => {
      const result = await execCli([], { autoClose: 3000 });

      // stdout carries the MCP protocol, and a host shows stderr as bridge
      // logs — neither may carry diagnostics nobody asked for.
      expect(result.stderr).not.toContain('PIERRE_SERVER_URL');
      expect(result.stderr).not.toContain('PIERRE_JWT_TOKEN');
    }, TIMEOUT);

    test('should log CI environment detection', async () => {
      const result = await execCli(['--verbose'], { autoClose: 3000 });

      // Should detect CI environment
      expect(result.stderr).toContain('CI');
    }, TIMEOUT);
  });

  describe('Error Handling', () => {
    test('should handle unknown options gracefully', async () => {
      const result = await execCli(['--unknown-option']);

      // Commander should show error for unknown option
      expect(result.code).not.toBe(0);
      expect(result.stderr).toContain('unknown option');
    }, TIMEOUT);

    test('should handle invalid timeout values', async () => {
      const result = await execCli(['--token-validation-timeout', 'not-a-number'], {
        autoClose: 3000,
      });

      // Should start but may have NaN issues
      // At minimum it shouldn't crash on parsing
      expect(typeof result.code).toBe('number');
    }, TIMEOUT);
  });

  describe('Graceful Shutdown', () => {
    test('should handle SIGTERM gracefully', async () => {
      const proc = spawn('node', [CLI_PATH], {
        env: {
          ...process.env,
          CI: 'true',
        },
      });

      let stderr = '';
      proc.stderr.on('data', (data) => {
        stderr += data.toString();
      });

      // Wait for startup
      await new Promise((resolve) => setTimeout(resolve, 2000));

      // Send SIGTERM
      proc.kill('SIGTERM');

      // Wait for shutdown
      await new Promise((resolve) => setTimeout(resolve, 2000));

      expect(stderr).toContain('shutting down');
    }, TIMEOUT);
  });

  describe('Combined Options', () => {
    test('should accept multiple options together', async () => {
      const result = await execCli([
        '--server', 'http://localhost:8081',
        '--oauth-client-id', 'client-id',
        '--callback-port', '35537',
        '--no-browser',
        '--help',
      ], { token: 'test-token' });

      expect(result.code).toBe(0);
    }, TIMEOUT);
  });
});
