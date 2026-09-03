// ABOUTME: Asserts the phone's settings list is derived from the shared pane declaration and serves every pane
// ABOUTME: Mobile shipped one long scroll while web shipped ten panes, and nothing compared the two

import fs from 'fs';
import path from 'path';
import { SETTINGS_PANES, settingsPanesFor } from '@pierre/shared-constants';

/**
 * Not a diff between web's tabs and mobile's rows. It checks one client
 * against the single declaration: adding a pane means editing
 * `SETTINGS_PANES` first, and this says whether mobile has caught up.
 */
const APP_DIR = path.join(__dirname, '..', 'app');
const SETTINGS_SCREEN = path.join(__dirname, '..', 'src', 'screens', 'settings', 'SettingsScreen.tsx');

/** Turn an expo-router path into the file that should serve it. */
function routeFileCandidates(route: string): string[] {
  const rel = route.replace(/^\//, '');
  return [
    path.join(APP_DIR, `${rel}.tsx`),
    path.join(APP_DIR, rel, 'index.tsx'),
    path.join(APP_DIR, rel, '_layout.tsx'),
  ];
}

describe('settings pane parity — mobile', () => {
  const mobilePanes = settingsPanesFor('mobile');

  it('serves every pane web serves', () => {
    // The audit's finding, as an assertion: web had ten named panes and the
    // phone had one scroll, so "privacy is missing on mobile" was a reasonable
    // reading of a screen where privacy sat 1,200pt down.
    const webOnly = SETTINGS_PANES.filter((pane) => pane.web !== null && pane.mobile === null);
    expect(webOnly.map((pane) => pane.id)).toEqual([]);
  });

  it('records a reason whenever a platform deliberately lacks a pane', () => {
    // A null without a reason is indistinguishable from an oversight.
    const unexplained = SETTINGS_PANES.filter(
      (pane) => (pane.web === null || pane.mobile === null) && !pane.why,
    );
    expect(unexplained.map((pane) => pane.id)).toEqual([]);
  });

  it.each(mobilePanes.map((pane) => [pane.id, pane.mobile as string]))(
    'implements %s at %s',
    (_id, route) => {
      const found = routeFileCandidates(route).some((candidate) => fs.existsSync(candidate));
      // jest's `expect` takes a single argument, so the explanation rides in
      // the compared value where the failure diff will actually print it.
      const outcome = found
        ? 'implemented'
        : `MISSING — no expo-router file serves ${route}. Build the pane, or set ` +
          'mobile: null in SETTINGS_PANES with a "why" if mobile should not have it.';
      expect(outcome).toBe('implemented');
    },
  );

  it('builds its rows from the declaration rather than a second hand-written list', () => {
    // A hand-written list is how the grouping drifted the first time: usage
    // stood alone here and sat inside Account on web, and both were correct
    // according to their own source.
    const source = fs.readFileSync(SETTINGS_SCREEN, 'utf8');
    expect(source).toContain("settingsPanesFor('mobile')");
    for (const pane of mobilePanes) {
      expect(source).not.toContain(`router.push('${pane.mobile}')`);
    }
  });

  it('no longer declares the retired AI-provider pane', () => {
    expect(SETTINGS_PANES.map((pane) => String(pane.id))).not.toContain('ai-provider');
    expect(SETTINGS_PANES.some((pane) => pane.mobile?.endsWith('/ai-provider'))).toBe(false);
    expect(fs.existsSync(path.join(APP_DIR, '(app)/(tabs)/(settings)/ai-provider.tsx'))).toBe(false);
  });
});
