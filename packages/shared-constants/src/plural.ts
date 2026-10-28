// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Picks the singular or plural catalogue key for a count, per locale, without Intl.PluralRules
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
