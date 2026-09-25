// ABOUTME: The phone's one type ladder — each step's size and line height in points (Boreal v2.2, DESIGN.md §10)
// ABOUTME: tailwind.config.js builds the `text-*` classes from it and inline styles read it through constants/theme

/* global module */

/**
 * Reading text is `base` 16/22; interface text is `sm` 13/18 at weight 500 on
 * anything navigable; `md` 15/20 is a row's second line when it is read, not
 * scanned (the chat preview, a Discover description); `lg` 17/22 is an inline
 * title in body; `xs` 12/16 is the floor — nothing goes under it but a native
 * badge. `2xl` and up are reserved for the auth and onboarding headline.
 *
 * CommonJS because the Tailwind config is: the class path requires this file
 * and the runtime path imports it, so the two cannot name a step differently.
 * Keys are React Native style names, so a step spreads into a style object.
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
