// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the published CLI contract - shebang, version, flags, quiet startup
// ABOUTME: Also pins the package manifest guarantees an external npm/npx consumer depends on

const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');

const SDK_ROOT = path.join(__dirname, '..', '..');
const CLI_PATH = path.join(SDK_ROOT, 'dist', 'cli.js');
const manifest = JSON.parse(fs.readFileSync(path.join(SDK_ROOT, 'package.json'), 'utf8'));

// A developer shell exports PIERRE_* variables (server URL, JWT, OAuth client) through
// .envrc, and the client now takes its credentials from exactly there - so every launch
// starts from an environment with those stripped and re-adds only what the case supplies.
// The browser kill switch is re-asserted because an OAuth launch would otherwise pop one.
const BASE_ENV = {
  ...Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('PIERRE_'))),
  PIERRE_DISABLE_BROWSER: 'true',
};

const runCli = (args, options = {}) =>
  spawnSync(process.execPath, [CLI_PATH, ...args], {
    encoding: 'utf8',
    timeout: options.timeout,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...BASE_ENV, ...options.env },
  });

// The bridge started by a bare launch runs until it is killed, so the launches that
// exercise startup output are bounded by a timeout and read from the collected pipes.
const LAUNCH_TIMEOUT_MS = 5000;

describe('Published bin', () => {
  test('is executable by node, not by bun', () => {
    const firstLine = fs.readFileSync(CLI_PATH, 'utf8').split('\n', 1)[0];

    expect(firstLine).toBe('#!/usr/bin/env node');
  });

  test('--version reports the package version', () => {
    const result = runCli(['--version']);

    expect(result.status).toBe(0);
    expect(result.stdout.trim()).toBe(manifest.version);
  });

  test('--help documents --verbose', () => {
    const result = runCli(['--help']);

    expect(result.status).toBe(0);
    expect(result.stdout).toContain('--verbose');
  });
});

describe('Credential flags', () => {
  // Process arguments are world-readable to any local process for as long as the client
  // runs (`ps -ef`, /proc/<pid>/cmdline) and they survive in shell history and in the MCP
  // host's configuration file, so no flag may carry a secret.
  test.each([
    ['--token', 'eyJhbGciOiJIUzI1NiJ9.leaked.jwt'],
    ['-t', 'eyJhbGciOiJIUzI1NiJ9.leaked.jwt'],
    ['--oauth-client-secret', 'leaked-client-secret'],
    ['--user-password', 'leaked-password'],
    ['--user-email', 'leaked@example.com'],
  ])('%s is not a flag the client accepts', (flag, value) => {
    const result = runCli([flag, value, '--server', 'http://127.0.0.1:1', '--no-browser'], {
      timeout: LAUNCH_TIMEOUT_MS,
    });

    expect(result.status).toBe(1);
    expect(result.stderr).toContain(`unknown option '${flag}'`);
    expect(result.stdout).toBe('');
  });

  test('--help documents the environment variables that replaced them', () => {
    const result = runCli(['--help']);

    expect(result.stdout).toContain('PIERRE_JWT_TOKEN');
    expect(result.stdout).toContain('PIERRE_OAUTH_CLIENT_SECRET');
    expect(result.stdout).not.toContain('--token <');
    expect(result.stdout).not.toContain('--oauth-client-secret');
    expect(result.stdout).not.toContain('--user-password');
  });
});

describe('Auth mode selection', () => {
  const launch = (env) =>
    runCli(['--verbose', '--server', 'http://127.0.0.1:1', '--no-browser'], {
      timeout: LAUNCH_TIMEOUT_MS,
      env,
    });

  test('PIERRE_JWT_TOKEN selects JWT mode without printing the token', () => {
    const result = launch({ PIERRE_JWT_TOKEN: 'eyJhbGciOiJIUzI1NiJ9.env.jwt' });

    expect(result.stderr).toContain('auth mode = jwt');
    expect(result.stderr).toContain('PIERRE_JWT_TOKEN = [SET]');
    expect(result.stderr).not.toContain('env.jwt');
  });

  test('PIERRE_OAUTH_CLIENT_SECRET selects the pre-registered OAuth client', () => {
    const result = launch({
      PIERRE_OAUTH_CLIENT_ID: 'client-from-env',
      PIERRE_OAUTH_CLIENT_SECRET: 'secret-from-env',
    });

    expect(result.stderr).toContain('auth mode = oauth');
    expect(result.stderr).toContain('OAuth client = client-from-env');
    expect(result.stderr).not.toContain('secret-from-env');
  });

  test('a client id with no secret in the environment falls back to dynamic registration', () => {
    const result = launch({ PIERRE_OAUTH_CLIENT_ID: 'client-from-env' });

    expect(result.stderr).toContain('auth mode = oauth');
    expect(result.stderr).toContain('OAuth client = [dynamic registration]');
  });
});

describe('Startup diagnostics', () => {
  test('are withheld from a plain launch', () => {
    const result = runCli(['--server', 'http://127.0.0.1:1', '--no-browser'], {
      timeout: LAUNCH_TIMEOUT_MS,
      env: { PIERRE_JWT_TOKEN: 'unit-test-token' },
    });

    expect(result.stderr).not.toContain('PIERRE_JWT_TOKEN');
    expect(result.stderr).not.toContain('NODE_ENV');
    expect(result.stdout).toBe('');
  });

  test('are printed to stderr under --verbose, never to the protocol stream', () => {
    const result = runCli(['--verbose', '--server', 'http://127.0.0.1:1', '--no-browser'], {
      timeout: LAUNCH_TIMEOUT_MS,
      env: { PIERRE_JWT_TOKEN: 'unit-test-token' },
    });

    expect(result.stderr).toContain(`pierre-mcp-client ${manifest.version} starting`);
    expect(result.stderr).toContain('server URL = http://127.0.0.1:1');
    expect(result.stderr).toContain('PIERRE_JWT_TOKEN = [SET]');
    expect(result.stderr).not.toContain('unit-test-token');
    expect(result.stdout).toBe('');
  });
});

describe('Package manifest', () => {
  test('ships the declarations its types entry points at', () => {
    expect(manifest.types).toBe('dist/index.d.ts');

    const declarations = fs.readFileSync(path.join(SDK_ROOT, manifest.types), 'utf8');
    expect(declarations).toContain('PierreMcpClient');
    expect(declarations).toContain('BridgeConfig');
  });

  test('publishes dist/ so the bin and the declarations reach consumers', () => {
    expect(manifest.files).toContain('dist/');
    expect(manifest.bin['pierre-mcp-client']).toBe('./dist/cli.js');
  });
});

describe('Bun preinstall guard', () => {
  const runGuard = (env) =>
    spawnSync('sh', ['-c', manifest.scripts.preinstall], {
      encoding: 'utf8',
      cwd: SDK_ROOT,
      env: { ...process.env, npm_execpath: '/usr/local/lib/node_modules/npm/bin/npm-cli.js', ...env },
    });

  test('rejects an npm install started in this directory', () => {
    const result = runGuard({ INIT_CWD: SDK_ROOT });

    expect(result.status).toBe(1);
    expect(result.stderr).toContain('Use bun');
  });

  test('stays silent when npm installs the package as a dependency', () => {
    // npm runs a dependency's preinstall with the cwd set to the installed package
    // directory while INIT_CWD stays at the consumer project that started the install.
    const result = runGuard({ INIT_CWD: os.tmpdir() });

    expect(result.status).toBe(0);
    expect(result.stderr).toBe('');
  });

  test('accepts a bun install started in this directory', () => {
    const result = runGuard({ INIT_CWD: SDK_ROOT, npm_execpath: '/opt/homebrew/bin/bun' });

    expect(result.status).toBe(0);
    expect(result.stderr).toBe('');
  });
});
