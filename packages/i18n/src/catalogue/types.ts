// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The shape of a client's string catalogue: what is embedded at build time, and how the rest loads
// ABOUTME: Mobile embeds all five locales; the web embeds French and fetches a chosen language's chunk on demand

/**
 * One locale's resources, under the single `translation` namespace. A type
 * alias, not an interface, so it stays assignable to i18next's indexed
 * `ResourceLanguage`.
 */
export type LocaleResources = {
  translation: Record<string, unknown>;
};

/** Resources keyed by locale tag. */
export type CatalogueResources = Record<string, LocaleResources>;

/** Reads one locale's catalogue file; resolves to the parsed `translation.json`. */
export type LocaleLoader = (locale: string) => Promise<Record<string, unknown>>;

/**
 * The catalogue a client starts with.
 *
 * `resources` is on screen before first paint and must hold the default
 * locale, which is also the fallback, so no key ever renders raw. `load`
 * supplies every other locale the first time it is chosen; absent, every
 * locale must already be in `resources`.
 */
export interface Catalogue {
  resources: CatalogueResources;
  load?: LocaleLoader;
}
