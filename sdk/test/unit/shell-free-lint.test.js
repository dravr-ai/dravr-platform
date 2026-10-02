// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Lints fixtures through the SDK's real ESLint config to prove the no-shell rules fire
// ABOUTME: A URL from a server's discovery document must never be parsed by a shell; the rule holds that for all of src/

const path = require('path');
const { restrictionHits } = require('../../../packages/eslint-config-pierre/testing/lintFixture.cjs');

const SDK = { cwd: path.resolve(__dirname, '..', '..'), file: 'src/fixture.ts' };
const hits = (source) => restrictionHits(SDK, source);

// Each fixture is linted by ESLint in its own process, on a shared runner.
jest.setTimeout(60_000);

// Every fixture names the one rule that must refuse it. Both rules say "No
// shell in the SDK", so a message alone stays green when the rule a fixture
// exists to prove is deleted and another happens to fire.
const IMPORTS = 'no-restricted-imports';
const SYNTAX = 'no-restricted-syntax';

const refusedOnceBy = async (source, ruleId, wording) => {
  const found = await hits(source);
  expect(found).toHaveLength(1);
  expect(found[0].ruleId).toBe(ruleId);
  expect(found[0].message).toContain(wording);
};

describe('sdk/src cannot reach a shell', () => {
  test.each([
    ['exec', 'import { exec } from "child_process";\nexport const open = (url: string) => exec(`open "${url}"`);'],
    ['execSync', 'import { execSync } from "child_process";\nexport const open = (url: string) => execSync("open " + url);'],
    ['exec under another name', 'import { exec as run } from "child_process";\nexport const open = (url: string) => run(`open "${url}"`);'],
    ['exec from the node: specifier', 'import { exec } from "node:child_process";\nexport const open = (url: string) => exec(`open "${url}"`);'],
  ])('refuses %s, the command-line APIs, at the import', async (_shape, source) => {
    await refusedOnceBy(source, IMPORTS, 'No shell in the SDK');
  });

  test('refuses child_process reached through a namespace import, which the import rule names as importing exec', async () => {
    await refusedOnceBy(
      'import * as cp from "child_process";\nexport const open = (url: string) => cp.exec(`open "${url}"`);',
      IMPORTS,
      'No shell in the SDK',
    );
  });

  test.each([
    ['a default import', 'import cp from "node:child_process";\nexport const open = (url: string) => cp.exec(`open "${url}"`);'],
    ['require', 'const cp = require("child_process");\nexport const open = (url: string) => cp.exec(`open "${url}"`);'],
    ['import-equals', 'import cp = require("child_process");\nexport const open = (url: string) => cp.exec(`open "${url}"`);'],
    ['a dynamic import', 'export const open = async (url: string) => (await import("child_process")).exec(`open "${url}"`);'],
  ])('refuses child_process reached through %s, which would hide exec from the import list', async (_shape, source) => {
    await refusedOnceBy(source, SYNTAX, 'Import child_process by name');
  });

  test.each([
    ['shell: true', 'import { execFile } from "child_process";\nexport const open = (url: string) => execFile("open", [url], { shell: true });'],
    ['a named shell', 'import { spawn } from "child_process";\nexport const open = (url: string) => spawn("open", [url], { shell: "/bin/sh" });'],
    ['a shell decided at runtime', 'import { spawn } from "child_process";\nexport const open = (url: string, shell: boolean) => spawn("open", [url], { shell });'],
    ['a quoted key', 'import { spawn } from "child_process";\nexport const open = (url: string) => spawn("open", [url], { "shell": true });'],
  ])('refuses %s on the argv APIs', async (_shape, source) => {
    const found = await hits(source);
    expect(found).toHaveLength(1);
    expect(found[0].ruleId).toBe(SYNTAX);
    // The shell entry's own message, not the by-name entry's, which also contains it.
    expect(found[0].message).toMatch(/^No shell in the SDK/);
  });

  test('accepts the launcher\'s shape: a named argv API, the URL as one argument, no shell', async () => {
    const source = [
      'import { execFile, spawn } from "child_process";',
      'export const open = (url: string) => execFile("open", [url], () => {});',
      'export const start = (url: string) => spawn("xdg-open", [url], { shell: false, stdio: "ignore" });',
      // A RegExp's exec is not a process: a text scan for `exec(` flags this line.
      'export const scheme = (url: string) => /^[a-z]+:/.exec(`${url}`);',
    ].join('\n');
    expect(await hits(source)).toEqual([]);
  });
});
