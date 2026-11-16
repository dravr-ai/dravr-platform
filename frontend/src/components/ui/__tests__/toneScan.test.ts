// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Scans frontend/src for a hue drawn as text on its own tint by hand, and for a pillar hue standing in for a state
// ABOUTME: Every such surface reads ALERT_TONE or BADGE_TONE, whose inks are measured; this catches the one written around them

import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = path.resolve(__dirname, '../../..');

function sources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== '__tests__' && entry.name !== 'test') sources(full, out);
    } else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) {
      out.push(full);
    }
  }
  return out;
}

/**
 * Every quoted string in a source, read twice: once a line at a time, and once
 * with a template literal spanning lines. The second pass alone would let one
 * stray backtick in a comment pair every string after it the wrong way round;
 * the first alone misses a class list broken over lines.
 */
function quoted(source: string): string[] {
  const singleLine = [...source.matchAll(/(["'`])((?:(?!\1)[^\\\n]|\\.)*)\1/g)].map((match) => match[2]);
  const multiLine = [...source.matchAll(/`((?:[^\\`]|\\.)*)`/g)].map((match) => match[1]).filter((s) => s.includes('\n'));
  return [...singleLine, ...multiLine];
}

/**
 * Every class list in a file: each quoted string, plus each `clsx(…)` call's
 * strings joined into one, since a call can put a tint and its ink in two
 * arguments.
 */
function strings(source: string): string[] {
  const calls = [...source.matchAll(/\bclsx\(/g)].map((match) => {
    let depth = 0;
    let end = match.index + match[0].length - 1;
    for (; end < source.length; end += 1) {
      if (source[end] === '(') depth += 1;
      else if (source[end] === ')' && (depth -= 1) === 0) break;
    }
    return quoted(source.slice(match.index, end + 1)).join(' ');
  });
  return [...quoted(source), ...calls];
}

const HUES = ['primary', 'error', 'success', 'warning', 'info', 'activity', 'nutrition', 'recovery', 'mobility'];

/*
 * Read because the rule is about class strings across every component, and no
 * lint sees a Tailwind class list: ESLint has no rule that pairs a `bg-<hue>/NN`
 * with a `text-<hue>` in the same list, and jsdom computes no styles to measure.
 */
const FILES = sources(SRC).map((file) => ({ file: path.relative(SRC, file), text: readFileSync(file, 'utf8') }));

describe('hand-written tone pairs', () => {
  it('draws no hue as text on its own tint, at rest or on hover, outside the tone tables', () => {
    // `hover:bg-error/10` under `text-error` counts: the label sits on that
    // tint the moment the pointer arrives (Button's danger-outline variant).
    const offenders: string[] = [];
    for (const { file, text } of FILES) {
      for (const value of strings(text)) {
        for (const hue of HUES) {
          const tint = new RegExp(`(^|[^\\w-])(bg|from|via|to)-${hue}/\\d+`);
          const ink = new RegExp(`(^|[^:\\w-])text-${hue}(?![\\w/-])`);
          if (tint.test(value) && ink.test(value)) offenders.push(`${file}: ${value.trim()}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it('puts no pillar hue on a filled button or a notice', () => {
    // A pillar marks a category and nothing else (DESIGN.md "Badges",
    // "Alerts"): an approve button is Button's success variant, a confirmation
    // is ALERT_TONE.success. The category tables (coachCategory, the avatar
    // slots, the scene's pillar dots) draw a pillar on its own, never beside
    // `text-on-primary` or as a bordered tint.
    const offenders: string[] = [];
    const pillarButton = /(^|[^:\w-])bg-(activity|nutrition|recovery|mobility)\b[^/][^]*text-on-primary/;
    const pillarNotice = /(^|[^:\w-])bg-(activity|nutrition|recovery|mobility)\/\d+[^]*border-(activity|nutrition|recovery|mobility)/;
    for (const { file, text } of FILES) {
      for (const value of strings(text)) {
        if (pillarButton.test(value) || pillarNotice.test(value)) offenders.push(`${file}: ${value.trim()}`);
      }
    }
    expect(offenders).toEqual([]);
  });

  it('draws no pillar hue as bare text', () => {
    // A pillar hue never stands as a word or glyph: a state, a link or a
    // bullet takes its role's ink, and a category labels in the bound
    // `text-on-<pillar>-container`.
    const offenders: string[] = [];
    const bare = /(^|[^:\w-])text-(activity|nutrition|recovery|mobility)(?![\w/-])/;
    for (const { file, text } of FILES) {
      for (const value of strings(text)) {
        if (bare.test(value)) offenders.push(`${file}: ${value.trim()}`);
      }
    }
    expect(offenders).toEqual([]);
  });
});
