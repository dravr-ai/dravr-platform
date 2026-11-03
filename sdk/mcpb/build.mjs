// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Stages and packs the Claude Desktop bundle (build/dravr-<version>.mcpb) from the built SDK
// ABOUTME: Keychain binaries are fetched per platform and checked against bun.lock's integrity before use

import { createHash } from 'crypto';
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'fs';
import { createRequire } from 'module';
import { join } from 'path';
import { spawnSync } from 'child_process';

const require = createRequire(import.meta.url);
const { KEYRING_PLATFORMS, SDK_ROOT, buildManifest, bundleFileName } = require('./bundle.cjs');

const BUILD_DIR = join(SDK_ROOT, 'build');
const STAGE_DIR = join(BUILD_DIR, 'mcpb');
const DIST_DIR = join(SDK_ROOT, 'dist');
const MCPB_BIN = join(SDK_ROOT, 'node_modules', '.bin', 'mcpb');
const ICON = join(SDK_ROOT, '..', 'frontend', 'public', 'pwa-512.png');
const KEYRING = '@napi-rs/keyring';
const REGISTRY = 'https://registry.npmjs.org';

const fail = (message) => {
  console.error(`❌ ${message}`);
  process.exit(1);
};

const run = (command, args) => {
  const result = spawnSync(command, args, { stdio: 'inherit' });
  if (result.error) {
    fail(`${command} could not start: ${result.error.message}`);
  }
  if (result.status !== 0) {
    fail(`${command} ${args.join(' ')} exited with ${result.status ?? result.signal}`);
  }
};

/**
 * Version and sha512 integrity of a package as bun.lock pins it.
 *
 * bun.lock lists each package as `"<name>": ["<name>@<version>", "<registry>", {meta}, "<integrity>"]`.
 * Reading the pin here means the bundle ships exactly the keychain build the SDK was tested with.
 */
const lockedPackage = (lock, name) => {
  const escaped = name.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&');
  const entry = new RegExp(`"${escaped}": \\["${escaped}@([^"]+)", "[^"]*", \\{[^\\n]*?\\}, "(sha512-[^"]+)"\\]`);
  const match = lock.match(entry);
  if (!match) {
    fail(`${name} has no pinned entry in bun.lock`);
  }
  return { version: match[1], integrity: match[2] };
};

/** Fetches a registry tarball, refuses it unless its sha512 matches the lockfile, and unpacks it. */
const fetchVerified = async (name, { version, integrity }, destination) => {
  const url = `${REGISTRY}/${name}/-/${name.split('/').pop()}-${version}.tgz`;
  const response = await fetch(url);
  if (!response.ok) {
    fail(`${url} answered ${response.status}`);
  }
  const tarball = Buffer.from(await response.arrayBuffer());
  const actual = `sha512-${createHash('sha512').update(tarball).digest('base64')}`;
  if (actual !== integrity) {
    fail(`${name}@${version} does not match bun.lock (expected ${integrity}, got ${actual})`);
  }
  const archive = join(BUILD_DIR, `${name.replace('/', '__')}-${version}.tgz`);
  writeFileSync(archive, tarball);
  mkdirSync(destination, { recursive: true });
  // npm tarballs hold everything under package/, which the module directory must not keep.
  run('tar', ['-xzf', archive, '-C', destination, '--strip-components=1']);
  rmSync(archive);
};

const required = [join(DIST_DIR, 'cli.js'), join(DIST_DIR, 'templates', 'oauth_success.html'), join(DIST_DIR, 'templates', 'oauth_error.html'), ICON, MCPB_BIN];
for (const file of required) {
  if (!existsSync(file)) {
    fail(`${file} is missing - run \`bun install\` and \`bun run build\` first`);
  }
}

const manifest = buildManifest();
console.log(`📦 Staging the Claude Desktop bundle for ${manifest.name} ${manifest.version}`);

rmSync(BUILD_DIR, { recursive: true, force: true });
mkdirSync(join(STAGE_DIR, 'server'), { recursive: true });

writeFileSync(join(STAGE_DIR, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
cpSync(ICON, join(STAGE_DIR, 'icon.png'));
cpSync(join(DIST_DIR, 'cli.js'), join(STAGE_DIR, 'server', 'cli.js'));
// The OAuth callback pages: oauth-session-manager reads them next to the running script.
cpSync(join(DIST_DIR, 'templates'), join(STAGE_DIR, 'server', 'templates'), { recursive: true });

// The keychain module stays outside the esbuild bundle (it is native), so it ships as
// node_modules at the bundle root, which `require` reaches by walking up from server/.
const lock = readFileSync(join(SDK_ROOT, 'bun.lock'), 'utf8');
const modules = join(STAGE_DIR, 'node_modules');
await fetchVerified(KEYRING, lockedPackage(lock, KEYRING), join(modules, KEYRING));
for (const { pkg } of KEYRING_PLATFORMS) {
  await fetchVerified(pkg, lockedPackage(lock, pkg), join(modules, pkg));
  console.log(`🔑 ${pkg}`);
}

// Smoke the staged tree the way Claude Desktop launches it, before packing: the entry point
// runs and reports this version, and the keychain module resolves from server/ and loads
// this machine's binary (one of the staged platforms on every machine that builds it).
const entry = join(STAGE_DIR, manifest.server.entry_point);
const launched = spawnSync(process.execPath, [entry, '--version'], { encoding: 'utf8' });
if (launched.status !== 0 || launched.stdout.trim() !== manifest.version) {
  fail(`${entry} --version answered ${JSON.stringify(launched.stdout)} (exit ${launched.status}): ${launched.stderr}`);
}
const { Entry } = createRequire(entry)(KEYRING);
if (typeof Entry !== 'function') {
  fail(`${KEYRING} resolved from ${entry} but exports no Entry`);
}
console.log(`🧪 ${manifest.server.entry_point} runs and loads the keychain on ${process.platform}-${process.arch}`);

run(MCPB_BIN, ['validate', join(STAGE_DIR, 'manifest.json')]);
const output = join(BUILD_DIR, bundleFileName(manifest.version));
run(MCPB_BIN, ['pack', STAGE_DIR, output]);
console.log(`✅ ${output}`);
