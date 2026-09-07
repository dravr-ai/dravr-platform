// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that no athlete-facing surface pins a colour the appearance setting cannot move
// ABOUTME: The retired gradient strip, the v1 primary literals and the second `warning` source each shipped for months

import { readFileSync, readdirSync, statSync } from 'fs';
import { join } from 'path';
import { SEMANTIC_COLORS, SEMANTIC_COLORS_DARK } from '@pierre/shared-constants';

const SRC = join(__dirname, '..', 'src');

/** Every .tsx/.ts under src/, recursively. */
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) return sources(full);
    return /\.tsx?$/.test(entry) ? [full] : [];
  });
}

/** Source with block and line comments stripped, so prose cannot satisfy a scan. */
function code(file: string): string {
  return readFileSync(file, 'utf8')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^\s*\/\/.*$/gm, '');
}

describe('no athlete-facing surface pins a colour the scheme cannot move', () => {
  it('offers no module-level palette a screen could reach for', () => {
    // `gradients` and `colors` were module-level `as const`s, so any component
    // drawing from either rendered the same colours in light and dark. Seven
    // screens took `gradients.violetCyan`; the settings screen took `colors`.
    // Pinning the exports rather than the call sites is the stronger claim: a
    // palette that does not exist cannot acquire a caller.
    const theme = code(join(SRC, 'constants', 'theme.ts'));
    expect(theme).not.toMatch(/export const (gradients|colors)\b/);
    // The live palette is the one way to a colour that follows the setting.
    expect(theme).toContain('useThemeColors');
  });

  it('draws no 3px gradient strip from the internal palette', () => {
    // DESIGN.md retired the gradient accent bar. Six survived on the phone —
    // the auth screens plus the chat list and connections.
    //
    // The rule is about the SOURCE, not the shape: SciotteLoginModal draws the
    // same 3px bar in a provider's own brand gradient, which §2 permits
    // ("third-party brand colors are not design-system violations") and which
    // is legitimately fixed — Strava's orange is Strava's orange in both
    // schemes. So this asserts no strip is drawn from `gradients.*`, our own
    // set, rather than maintaining a list of files allowed to have one.
    const offenders = sources(SRC).filter((f) => {
      const src = code(f);
      return /height: 3, width: '100%'/.test(src) && /colors=\{gradients\./.test(src);
    });
    expect(offenders).toEqual([]);
  });

  it('carries no retired v1 primary literal', () => {
    // #00241a is the Boreal v1 primary, retired because it read as black at
    // every size. It survived as a category colour and inside a login gradient.
    const offenders = sources(SRC).filter((f) => /#00241a|#0d3b2e/i.test(code(f)));
    expect(offenders).toEqual([]);
  });

  it('resolves every feedback colour from the one shared set', () => {
    // The phone answered with two different ambers for `warning` depending on
    // whether a component read a NativeWind class or useThemeColors().
    const themeContext = code(join(SRC, 'contexts', 'ThemeContext.tsx'));
    expect(themeContext).toContain('SEMANTIC_COLORS');
    // The Editorial-tier value that had drifted in here.
    expect(themeContext).not.toContain('#8f6a2e');

    // And the shared set is the Product tier, so the fix points at the right one.
    expect(SEMANTIC_COLORS.warning).toBe('#b08326');
    expect(SEMANTIC_COLORS_DARK.warning).toBe('#d6b87a');
  });
});
