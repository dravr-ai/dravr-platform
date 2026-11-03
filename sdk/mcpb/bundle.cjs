// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Single source for the Claude Desktop bundle (.mcpb): its manifest and the keyring binaries it ships
// ABOUTME: The version comes from package.json and the platform list from the binaries, so neither is kept twice

const fs = require('fs');
const path = require('path');

const SDK_ROOT = path.join(__dirname, '..');

/**
 * The native OS-keychain binaries the bundle ships, one per platform Claude Desktop runs on.
 *
 * `@napi-rs/keyring` picks its binary at runtime from these optional packages, and a bundle
 * is a single archive for every platform, so each one is staged rather than only the one the
 * build machine installed. A platform missing here still runs: secure-storage falls back to
 * owner-only file storage when the keychain module does not load.
 */
const KEYRING_PLATFORMS = [
  { os: 'darwin', pkg: '@napi-rs/keyring-darwin-arm64' },
  { os: 'darwin', pkg: '@napi-rs/keyring-darwin-x64' },
  { os: 'win32', pkg: '@napi-rs/keyring-win32-x64-msvc' },
  { os: 'win32', pkg: '@napi-rs/keyring-win32-arm64-msvc' },
  { os: 'linux', pkg: '@napi-rs/keyring-linux-x64-gnu' },
  { os: 'linux', pkg: '@napi-rs/keyring-linux-arm64-gnu' },
];

/** The archive name the build writes under build/, for a given package version. */
const bundleFileName = (version) => `dravr-${version}.mcpb`;

/**
 * The bundle manifest: sdk/mcpb/manifest.json plus what is derived rather than written down.
 *
 * `version` is package.json's, so the release workflow's tag==version check covers the bundle
 * too. `compatibility.platforms` is the set of platforms a keychain binary is staged for.
 */
const buildManifest = () => {
  const template = JSON.parse(fs.readFileSync(path.join(__dirname, 'manifest.json'), 'utf8'));
  const pkg = JSON.parse(fs.readFileSync(path.join(SDK_ROOT, 'package.json'), 'utf8'));
  const platforms = [...new Set(KEYRING_PLATFORMS.map(({ os }) => os))];
  return {
    ...template,
    version: pkg.version,
    compatibility: { ...template.compatibility, platforms },
  };
};

module.exports = { KEYRING_PLATFORMS, SDK_ROOT, buildManifest, bundleFileName };
