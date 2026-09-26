// ABOUTME: The keyless OpenFreeMap basemap both route cards draw over, one sheet per colour scheme
// ABOUTME: Web's MapLibre GL and the phone's MapLibre Native read the same two style URLs from here

import type { ColorScheme } from './design-system';

/**
 * OpenFreeMap serves the vector basemap and its glyph ranges without a key and
 * without an account, which is why it is the basemap here: a route card that
 * needed a vendor token could not render for an athlete at all until someone
 * provisioned one.
 *
 * Two styles rather than one, because a map is the largest block of colour the
 * thread ever shows and a paper-white basemap on the near-black canvas is a
 * lamp. `positron` is the quietest style OpenFreeMap publishes — a desaturated
 * ground that leaves the track as the only saturated thing on it — and `dark`
 * is its counterpart. Both carry the same glyphs endpoint, so labels resolve
 * either way.
 */
export const BASEMAP_STYLE: Record<ColorScheme, string> = {
  light: 'https://tiles.openfreemap.org/styles/positron',
  dark: 'https://tiles.openfreemap.org/styles/dark',
};
