// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Scans the phone's source for a hue drawn on its own tint by hand — a `${colour}NN` or `colour + 'NN'` hex-alpha fill, or a `/NN` tint beside its own hue
// ABOUTME: Every such surface reads ALERT_TONE or BADGE_TONE, whose inks are measured; planted offenders prove the scan sees every shape

import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'fs';
import { tmpdir } from 'os';
import { join, relative, resolve } from 'path';

const SRC = resolve(__dirname, '../../..');

const HUES = ['primary', 'error', 'success', 'warning', 'info', 'activity', 'nutrition', 'recovery', 'mobility'];

function sources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== '__tests__' && entry.name !== 'test') sources(full, out);
    } else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) {
      out.push(full);
    }
  }
  return out;
}

/**
 * A colour value with a two-digit alpha glued on: `${colors.pierre.violet}26`.
 * Built from a whole colour, never from a bare channel string — `#${rgb}00`,
 * a gradient stop composing its own hex, is not one.
 */
const HEX_ALPHA = /(^|[^#])\$\{[^}]+\}[0-9a-fA-F]{2}`/;

/** The same, glued by concatenation: `colour + '20'`, `colors.x + "1A"`. */
const HEX_ALPHA_CONCAT = /\+\s*(['"`])[0-9a-fA-F]{2}\1/;

/** Every quoted or template string in a file — a class list is one of them. */
function strings(source: string): string[] {
  return [...source.matchAll(/(["'`])((?:(?!\1)[^\\]|\\.)*)\1/g)].map((match) => match[2]);
}

/** A string that tints with a hue and labels in the same hue. */
function pairsHueWithOwnTint(value: string): boolean {
  return HUES.some(
    (hue) =>
      new RegExp(`(^|[^\\w-])bg-${hue}/\\d+`).test(value) && new RegExp(`(^|[^:\\w-])text-${hue}(?![\\w/-])`).test(value),
  );
}

/**
 * Every hand-drawn tint under `root`, as `file: line`. It reads source because
 * this is a rule across the codebase that no lint expresses: a hue built into
 * a template string, or paired with its own tint inside one class list.
 */
function scan(root: string): string[] {
  const offenders: string[] = [];
  for (const file of sources(root)) {
    const text = readFileSync(file, 'utf8');
    for (const line of text.split('\n')) {
      if (HEX_ALPHA.test(line) || HEX_ALPHA_CONCAT.test(line)) offenders.push(`${relative(root, file)}: ${line.trim()}`);
    }
    for (const value of strings(text)) {
      if (pairsHueWithOwnTint(value)) offenders.push(`${relative(root, file)}: ${value.trim()}`);
    }
  }
  return offenders;
}

describe('hand-drawn tints on the phone', () => {
  it('draws no hex-alpha fill and no hue on its own `/NN` tint outside the tone tables', () => {
    expect(scan(SRC)).toEqual([]);
  });

  it('catches every shape when it is planted', () => {
    const root = mkdtempSync(join(tmpdir(), 'tone-scan-'));
    try {
      writeFileSync(
        join(root, 'Planted.tsx'),
        [
          'const a = <View style={{ backgroundColor: `${colors.pierre.violet}20` }} />;',
          'const b = <Text className="bg-error/10 text-error">x</Text>;',
          'const c = <View style={{ backgroundColor: colors.pierre.violet + \'20\' }} />;',
          'const d = <View style={{ backgroundColor: accent + "1A" }} />;',
          'const fine = `#${rgb}00`;',
          'const alsoFine = <Text className="bg-error/10 text-on-error-container">x</Text>;',
        ].join('\n'),
      );
      const found = scan(root);
      expect(found).toHaveLength(4);
      expect(found[0]).toContain('${colors.pierre.violet}20');
      expect(found[1]).toContain("colors.pierre.violet + '20'");
      expect(found[2]).toContain('accent + "1A"');
      expect(found[3]).toContain('bg-error/10 text-error');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
