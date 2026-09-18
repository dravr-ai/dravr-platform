// ABOUTME: Every locale carries exactly the same key set, so no string can ship in one language only
// ABOUTME: The chrome defaults to fr, so an en-only key is invisible in review and visible to the athlete

import { readFileSync } from 'fs';
import { join } from 'path';

type Catalogue = Record<string, unknown>;

const LOCALE_DIR = join(__dirname, '..', '..', '..', 'packages', 'i18n', 'src', 'locales');
const NAMES = ['de', 'en', 'es', 'fr', 'pt'] as const;
const REFERENCE = 'en';

function load(locale: string): Catalogue {
  return JSON.parse(
    readFileSync(join(LOCALE_DIR, locale, 'translation.json'), 'utf8'),
  ) as Catalogue;
}

const LOCALES: Record<string, Catalogue> = Object.fromEntries(
  NAMES.map((n) => [n, load(n)]),
);

/** Every leaf key, dotted, so a nested namespace is compared as precisely as a top-level one. */
function leafKeys(node: unknown, prefix = ''): string[] {
  if (node === null || typeof node !== 'object' || Array.isArray(node)) return [prefix];
  return Object.entries(node as Catalogue).flatMap(([k, v]) =>
    leafKeys(v, prefix ? `${prefix}.${k}` : k),
  );
}

describe('i18n locale parity', () => {
  const reference = new Set(leafKeys(LOCALES[REFERENCE]));

  it('reads a real catalogue, so an empty read cannot pass as agreement', () => {
    // Without this, a failed read would make every comparison below trivially
    // true and the whole file would be a green no-op.
    expect(reference.size).toBeGreaterThan(500);
  });

  it.each(NAMES.filter((l) => l !== REFERENCE))('%s has exactly the keys en has', (locale) => {
    const theirs = new Set(leafKeys(LOCALES[locale]));
    const missing = [...reference].filter((k) => !theirs.has(k)).sort();
    const extra = [...theirs].filter((k) => !reference.has(k)).sort();
    expect({ missing, extra }).toEqual({ missing: [], extra: [] });
  });

  it.each(NAMES)('%s has no blank value', (locale) => {
    const blanks: string[] = [];
    const walk = (node: unknown, path = ''): void => {
      if (typeof node === 'string') {
        if (node.trim() === '') blanks.push(path);
        return;
      }
      if (node && typeof node === 'object' && !Array.isArray(node)) {
        for (const [k, v] of Object.entries(node as Catalogue)) {
          walk(v, path ? `${path}.${k}` : k);
        }
      }
    };
    walk(LOCALES[locale]);
    expect(blanks).toEqual([]);
  });
});
