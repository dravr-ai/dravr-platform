// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Claude Desktop bundle manifest to the SDK it ships: schema-valid, same version, real flags
// ABOUTME: Also pins the staged keychain platforms to the ones the locked @napi-rs/keyring actually publishes

const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');

const { KEYRING_PLATFORMS, SDK_ROOT, buildManifest, bundleFileName } = require('../../mcpb/bundle.cjs');

const MCPB_BIN = path.join(SDK_ROOT, 'node_modules', '.bin', 'mcpb');
const CLI_PATH = path.join(SDK_ROOT, 'dist', 'cli.js');
// Why this reads a file: package.json carries the version every published artifact must share.
const pkg = JSON.parse(fs.readFileSync(path.join(SDK_ROOT, 'package.json'), 'utf8'));

describe('Claude Desktop bundle manifest', () => {
  const manifest = buildManifest();

  test('passes the official mcpb validator', () => {
    // The validator also checks the icon next to the manifest, so the manifest is written
    // beside a copy of the icon the build stages - the same layout the bundle has.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mcpb-manifest-'));
    try {
      fs.writeFileSync(path.join(dir, 'manifest.json'), JSON.stringify(manifest));
      fs.copyFileSync(path.join(SDK_ROOT, '..', 'frontend', 'public', 'pwa-512.png'), path.join(dir, manifest.icon));

      const result = spawnSync(MCPB_BIN, ['validate', path.join(dir, 'manifest.json')], { encoding: 'utf8' });

      expect(result.stderr).not.toMatch(/ERROR/);
      expect(result.status).toBe(0);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('carries the package version, so the release tag check covers the bundle', () => {
    expect(manifest.version).toBe(pkg.version);
  });

  test('is written under the name the release workflow uploads and attaches', () => {
    // Why this reads a file: parity check - sdk-release.yml names the archive build.mjs writes,
    // and nothing at runtime joins the two, so a rename on either side fails here.
    const workflow = fs.readFileSync(path.join(SDK_ROOT, '..', '.github', 'workflows', 'sdk-release.yml'), 'utf8');

    expect(workflow).toContain(`path: sdk/build/${bundleFileName('${{ needs.validate.outputs.version }}')}`);
    expect(workflow).toContain(`"mcpb/${bundleFileName('${VERSION}')}"`);
  });

  test('launches the entry point it declares, with node', () => {
    const { entry_point: entryPoint, mcp_config: config } = manifest.server;

    expect(manifest.server.type).toBe('node');
    expect(config.command).toBe('node');
    expect(config.args[0]).toBe(`\${__dirname}/${entryPoint}`);
  });

  test('passes the server URL through a flag the CLI defines', () => {
    const { args } = manifest.server.mcp_config;
    const flag = args[args.indexOf('${user_config.server_url}') - 1];
    const help = spawnSync(process.execPath, [CLI_PATH, '--help'], { encoding: 'utf8' });

    expect(flag).toBe('--server');
    expect(help.stdout).toContain(`${flag} <url>`);
  });

  test('defaults to the public Dravr MCP origin and sends no credential through config', () => {
    const { server_url: serverUrl } = manifest.user_config;

    // Wire value: the public origin every installed bundle connects to by default.
    expect(serverUrl.default).toBe('https://mcp.dravr.ai');
    expect(Object.values(manifest.user_config).some((field) => field.sensitive)).toBe(false);
    expect(manifest.server.mcp_config.env).toBeUndefined();
  });

  test('lists a privacy policy, which the Claude directory requires of a local connector', () => {
    // Wire value: the published policy page the directory links to users.
    expect(manifest.privacy_policies).toEqual(['https://dravr.ai/privacy']);
  });

  test('claims both Claude Desktop OSes, each with a keychain binary for x64 and arm64', () => {
    const { platforms } = manifest.compatibility;

    // Wire values: the OSes Claude Desktop ships on, named as the mcpb schema spells them.
    expect(platforms).toEqual(expect.arrayContaining(['darwin', 'win32']));
    for (const platform of platforms) {
      const staged = KEYRING_PLATFORMS.filter(({ os: stagedOs }) => stagedOs === platform).map(({ pkg: stagedPkg }) => stagedPkg);
      expect(staged.some((name) => name.includes('-x64'))).toBe(true);
      expect(staged.some((name) => name.includes('-arm64'))).toBe(true);
    }
  });
});

describe('Staged keychain binaries', () => {
  test('are platform packages the locked @napi-rs/keyring publishes, at its version', () => {
    // Why this reads a file: the installed keyring's package.json names the platform packages
    // its loader requires, so a keyring bump that renames or drops one fails here, not at release.
    const keyring = JSON.parse(
      fs.readFileSync(path.join(SDK_ROOT, 'node_modules', '@napi-rs', 'keyring', 'package.json'), 'utf8'),
    );

    for (const { pkg: platformPackage } of KEYRING_PLATFORMS) {
      expect(keyring.optionalDependencies).toHaveProperty([platformPackage], keyring.version);
    }
  });
});
