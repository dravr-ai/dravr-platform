// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Several items as one phrase in the reader's language — "Garmin and Strava", "Garmin et Strava"
// ABOUTME: Intl.ListFormat where the runtime carries it; the phone's engine does not, so it joins with a comma there

/** The part of `Intl.ListFormat` this module calls. */
interface ListFormatter {
  format(items: readonly string[]): string;
}

/** `Intl.ListFormat`'s constructor, typed for the one shape this module builds. */
type ListFormatConstructor = new (
  locale: string,
  options: { style: 'long'; type: 'conjunction' },
) => ListFormatter;

/**
 * The runtime's `Intl.ListFormat`, or `undefined` where it has none. Read
 * through a cast because the shared packages compile against ES2020, whose
 * library has no type for it, while browsers have shipped it since 2021.
 */
function listFormatConstructor(): ListFormatConstructor | undefined {
  return (Intl as unknown as { ListFormat?: ListFormatConstructor }).ListFormat;
}

/**
 * `items` as one phrase in `locale`: "Garmin, Strava, and COROS" in English,
 * "Garmin, Strava und COROS" in German. On a runtime without
 * `Intl.ListFormat` — Hermes, the phone's engine — the items are joined with
 * a comma, which every locale reads.
 */
export function formatList(items: readonly string[], locale: string): string {
  const ListFormat = listFormatConstructor();
  if (ListFormat === undefined) {
    return items.join(', ');
  }
  return new ListFormat(locale, { style: 'long', type: 'conjunction' }).format(items);
}
