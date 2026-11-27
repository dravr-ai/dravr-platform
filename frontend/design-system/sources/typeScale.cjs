// ABOUTME: The phone's type ladder — each step's size and line height in points (Boreal v2.2)
// ABOUTME: Copied from the app in dravr-pitoune; the design-system build reads it for the phone tokens

/* global module */

/**
 * Reading text is `base` 16/22; interface text is `sm` 13/18 at weight 500 on
 * anything navigable; `md` 15/20 is a row's second line when it is read, not
 * scanned (the chat preview, a Discover description); `lg` 17/22 is an inline
 * title in body; `xs` 12/16 is the floor — nothing goes under it but a native
 * badge. `2xl` and up are reserved for the auth and onboarding headline.
 *
 * CommonJS because frontend/scripts/design-system/sources.ts loads it with
 * require() from an ES-module package. Keys are React Native style names, as
 * in the app's own ladder.
 */
const TYPE_SCALE = {
  xs: { fontSize: 12, lineHeight: 16 },
  sm: { fontSize: 13, lineHeight: 18 },
  md: { fontSize: 15, lineHeight: 20 },
  base: { fontSize: 16, lineHeight: 22 },
  lg: { fontSize: 17, lineHeight: 22 },
  xl: { fontSize: 20, lineHeight: 25 },
  '2xl': { fontSize: 22, lineHeight: 28 },
  '3xl': { fontSize: 26, lineHeight: 32 },
};

module.exports = { TYPE_SCALE };
