// ABOUTME: Tests for package.sh: the store zip holds every file the extension loads and nothing else
// ABOUTME: Packs a copy of the extension into a temporary folder under `bun test extensions/dravr-signin` (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { cpSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'bun:test';

const HERE = import.meta.dir;

// The manifest is read as data: the files it names are the ones the browser
// loads, so the zip must carry each of them.
const MANIFEST = JSON.parse(readFileSync(join(HERE, 'manifest.json'), 'utf8'));

/** Every file the browser loads from the package. */
function loadedFiles() {
  const background = [MANIFEST.background.service_worker, ...MANIFEST.background.scripts];
  const contentScripts = MANIFEST.content_scripts.flatMap((script) => script.js);
  const icons = Object.values(MANIFEST.icons);
  const locales = readdirSync(join(HERE, '_locales')).map((locale) => `_locales/${locale}/messages.json`);
  // background.js opens notice.html, which loads notice.js; both import signin.js.
  const pages = ['notice.html', 'notice.js', 'signin.js'];
  return [...new Set(['manifest.json', ...background, ...contentScripts, ...icons, ...locales, ...pages])].sort();
}

let work = null;

afterEach(() => {
  if (work) rmSync(work, { recursive: true, force: true });
  work = null;
});

/** Pack a copy of the extension (plus `extra` files written into it) and list the zip's files. */
function pack(extra = {}) {
  work = mkdtempSync(join(tmpdir(), 'dravr-signin-'));
  const copy = join(work, 'extension');
  cpSync(HERE, copy, { recursive: true, filter: (source) => !source.startsWith(join(HERE, 'dist')) });
  for (const [path, content] of Object.entries(extra)) writeFileSync(join(copy, path), content);

  const packed = Bun.spawnSync(['bash', join(copy, 'package.sh'), join(work, 'out')]);
  expect(packed.exitCode).toBe(0);
  const zipPath = packed.stdout.toString().trim();
  expect(zipPath).toBe(join(work, 'out', `dravr-signin-${MANIFEST.version}.zip`));

  const listed = Bun.spawnSync(['unzip', '-Z1', zipPath]);
  expect(listed.exitCode).toBe(0);
  return listed.stdout
    .toString()
    .split('\n')
    .filter((entry) => entry && !entry.endsWith('/'))
    .sort();
}

describe('package.sh', () => {
  it('packs exactly the files the browser loads, named after the manifest\'s version', () => {
    expect(pack()).toEqual(loadedFiles());
  });

  it('leaves out a hidden file in the packed folders', () => {
    const files = pack({ 'icons/.DS_Store': 'finder', '_locales/.DS_Store': 'finder' });
    expect(files.filter((file) => file.split('/').some((part) => part.startsWith('.')))).toEqual([]);
    expect(files).toEqual(loadedFiles());
  });
});
