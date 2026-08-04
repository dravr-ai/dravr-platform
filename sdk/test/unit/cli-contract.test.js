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

const runCli = (args, options = {}) =>
  spawnSync(process.execPath, [CLI_PATH, ...args], {
    encoding: 'utf8',
    timeout: options.timeout,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, ...options.env },
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
