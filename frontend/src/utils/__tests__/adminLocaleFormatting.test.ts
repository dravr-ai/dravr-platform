// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: An untranslated component formats its dates and numbers in ADMIN_LOCALE, never the browser's
// ABOUTME: Its text is English whatever the session, so a browser-locale date put "09 h 13" beside it (carnet#1462)

import { readdirSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = path.resolve(__dirname, '../..');

/** Every .ts/.tsx under `dir` that is not a test. */
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) return name === '__tests__' ? [] : sources(full);
    return /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) ? [full] : [];
  });
}

/**
 * A format call that leaves the locale to the browser: no argument, `undefined`
 * or an empty list, with the argument on the next line when a formatter wrapped it.
 */
const BROWSER_LOCALE_FORMAT = /\.toLocale(?:String|DateString|TimeString)\(\s*(?:\)|undefined\b|\[\s*\])/g;
/** A component that renders through the catalogue, so the viewer's locale is its own. */
const TRANSLATED = /\buseTranslation\b|from ['"]@pierre\/i18n['"]|from ['"]i18next['"]/;

/** The `file:line` of each browser-locale format in a source no catalogue translates. */
function browserLocaleFormats(file: string, source: string): string[] {
  if (TRANSLATED.test(source)) return [];
  return [...source.matchAll(BROWSER_LOCALE_FORMAT)].map(
    (m) => `${file}:${source.slice(0, m.index).split('\n').length}`,
  );
}

describe('formatting on untranslated screens', () => {
  it('flags a browser-locale format only in an untranslated source', () => {
    expect(browserLocaleFormats('a.tsx', 'x\nconst d = new Date(t).toLocaleString();')).toEqual(['a.tsx:2']);
    expect(browserLocaleFormats('a.tsx', 'n.toLocaleString(undefined, { maximumFractionDigits: 1 })')).toEqual([
      'a.tsx:1',
    ]);
    expect(browserLocaleFormats('a.tsx', 'n.toLocaleString(\n  undefined,\n  { maximumFractionDigits: 1 },\n)')).toEqual([
      'a.tsx:1',
    ]);
    expect(browserLocaleFormats('a.tsx', "d.toLocaleTimeString([], { hour: '2-digit' })")).toEqual(['a.tsx:1']);
    expect(browserLocaleFormats('a.tsx', 'n.toLocaleString(ADMIN_LOCALE)')).toEqual([]);
    expect(browserLocaleFormats('a.tsx', 'n.toLocaleString(\n  ADMIN_LOCALE,\n)')).toEqual([]);
    expect(
      browserLocaleFormats('a.tsx', "import { useTranslation } from '@pierre/i18n';\nn.toLocaleString();"),
    ).toEqual([]);
  });

  it('has none in the web sources', () => {
    // Reading the sources is the rule itself: a codebase-wide scan for a call no
    // lint can express (whether a file is translated decides if the call is wrong).
    const untranslated = sources(SRC)
      .map((file) => ({ file: path.relative(SRC, file), source: readFileSync(file, 'utf8') }))
      .filter(({ source }) => !TRANSLATED.test(source));
    // Fail closed: a scan that reached none of the admin screens verified nothing.
    expect(
      untranslated.some(({ source }) => source.includes('toLocaleString(ADMIN_LOCALE')),
      'the scan must reach the untranslated admin components it guards',
    ).toBe(true);
    const found = untranslated.flatMap(({ file, source }) => browserLocaleFormats(file, source));
    expect(found).toEqual([]);
  });
});
