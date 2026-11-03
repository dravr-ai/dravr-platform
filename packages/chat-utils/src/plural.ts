// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Picks the singular or plural, or the ordinal, catalogue key for a count, per locale, without Intl.PluralRules
// ABOUTME: French and Portuguese read 0 as singular (« 0 signalée »); English, Spanish and German read it as plural

/** A count's two catalogue keys: `{{count}} signalée` / `{{count}} signalées`. */
export interface CountKeys {
  one: string;
  many: string;
}

/**
 * The languages whose singular covers 0 as well as 1, by base language tag.
 * CLDR's `one` category for French is i = 0,1 and for Portuguese (pt, the
 * Brazilian default the catalogue ships) i = 0..1; English, Spanish and
 * German keep 0 plural.
 */
const ZERO_IS_SINGULAR: ReadonlySet<string> = new Set(['fr', 'pt']);

/**
 * The catalogue key a count reads under in `language`.
 *
 * The rule is written out rather than asked of `Intl.PluralRules`, which the
 * phone's JavaScript engine does not ship in full (carnet#227 crashed the
 * notification list on a missing `Intl` subset). Only whole, non-negative
 * counts reach it; a fractional count reads plural everywhere. An unknown or
 * missing language falls back to the English rule.
 */
export function countKey(count: number, language: string | undefined, keys: CountKeys): string {
  if (count === 1) return keys.one;
  if (count === 0 && ZERO_IS_SINGULAR.has(baseLanguage(language))) return keys.one;
  return keys.many;
}

/** `fr-CA` → `fr`; the catalogue's languages differ only by their base tag. */
function baseLanguage(language: string | undefined): string {
  return (language ?? '').toLowerCase().split(/[-_]/, 1)[0] ?? '';
}

/** The CLDR ordinal categories the catalogue writes a `_ordinal_*` key for. */
export type OrdinalCategory = 'one' | 'two' | 'few' | 'other';

/**
 * The CLDR ordinal category of a whole count in `language`, written out for
 * the same reason `countKey` is: `Intl.PluralRules` with `type: 'ordinal'` is
 * not there on every runtime the phone ships. English: 1st, 2nd, 3rd, 4th,
 * 11th–13th, 21st…; French: 1er, then 2e…; Spanish, German and Portuguese
 * write one ordinal mark for every count. An unknown language reads English.
 */
export function ordinalCategory(count: number, language: string | undefined): OrdinalCategory {
  const base = baseLanguage(language);
  if (base === 'fr') return count === 1 ? 'one' : 'other';
  if (base === 'es' || base === 'de' || base === 'pt') return 'other';
  const tens = count % 100;
  if (tens >= 11 && tens <= 13) return 'other';
  switch (count % 10) {
    case 1:
      return 'one';
    case 2:
      return 'two';
    case 3:
      return 'few';
    default:
      return 'other';
  }
}

/** The `_ordinal_<category>` twin of `key` a count reads under in `language` (« the 4th morning »). */
export function ordinalKey(count: number, language: string | undefined, key: string): string {
  return `${key}_ordinal_${ordinalCategory(count, language)}`;
}
