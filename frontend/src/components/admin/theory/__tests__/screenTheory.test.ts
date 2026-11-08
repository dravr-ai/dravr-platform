// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Keeps the admin screen theory current: one entry per admin tab, and every cited commit real, on HEAD's history, dated and titled as git says
// ABOUTME: Runs git through child_process and fails, never skips, when git cannot answer (CI checks out full history for it)

import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { ADMIN_ONLY_TABS, SUPER_ADMIN_ONLY_TABS } from '../../../../constants/adminTabs';
import { CITED_COMMITS, SCREEN_THEORY } from '../screenTheory';

const HERE = path.dirname(fileURLToPath(import.meta.url));

/** The repository's root commit: an import, never a screen's origin (carnet#1441). */
const ROOT_IMPORT = '1b33079e0';

/** Admin screens reached off the rail: the operator's own settings. */
const OFF_RAIL_ADMIN_SCREENS = ['settings'];

function git(...args: string[]): string {
  return execFileSync('git', args, { cwd: HERE, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
}

const entries = Object.entries(SCREEN_THEORY);
const cited = entries.flatMap(([tab, entry]) => entry.commits.map((commit) => ({ tab, commit })));

describe('admin screen theory coverage', () => {
  it('has an entry for every admin screen and none for anything else', () => {
    const expected = new Set([...ADMIN_ONLY_TABS, ...SUPER_ADMIN_ONLY_TABS, ...OFF_RAIL_ADMIN_SCREENS]);
    expect(new Set(Object.keys(SCREEN_THEORY))).toEqual(expected);
  });

  it.each(entries)('%s says what it answers, how, since when and with which commits', (_tab, entry) => {
    expect(entry.question.trim()).not.toBe('');
    expect(entry.theory.trim()).not.toBe('');
    expect(entry.computation.length).toBeGreaterThan(0);
    expect(entry.commits.length).toBeGreaterThan(0);
    expect(entry.since).toMatch(/^(before 2026-05-05|\d{4}-\d{2}-\d{2})$/);
  });

  it.each(entries.filter(([, entry]) => entry.since !== 'before 2026-05-05'))(
    '%s dates itself by a commit it cites',
    (_tab, entry) => {
      expect(entry.commits.map((commit) => commit.date)).toContain(entry.since);
    },
  );

  it('cites every commit in the table at least once', () => {
    const used = new Set(cited.map(({ commit }) => commit.sha));
    expect(CITED_COMMITS.filter((commit) => !used.has(commit.sha)).map((commit) => commit.sha)).toEqual([]);
  });
});

describe('admin screen theory commits', () => {
  it.each(CITED_COMMITS.map((commit) => [commit.sha, commit] as const))(
    '%s resolves, is on HEAD\'s history, and matches its date and subject',
    (sha, commit) => {
      expect(sha).toMatch(/^[0-9a-f]{9,}$/);
      expect(sha.startsWith(ROOT_IMPORT)).toBe(false);
      // Each call throws on a non-zero exit, which fails the test: a missing
      // git, a shallow clone or an unknown SHA is a failure, never a skip.
      git('cat-file', '-e', `${sha}^{commit}`);
      git('merge-base', '--is-ancestor', sha, 'HEAD');
      expect(git('show', '-s', '--format=%as%x09%s', sha).replace(/\n$/, '')).toBe(`${commit.date}\t${commit.subject}`);
    },
  );
});
