// ABOUTME: Asserts mobile implements every surface the shared registry declares for it
// ABOUTME: Checks real expo-router files on disk, so a declared-but-unbuilt screen fails here

import fs from 'fs';
import path from 'path';
import { USER_SURFACES, surfacesFor } from '@pierre/shared-constants';

/**
 * Mobile silently lacked Profile, Privacy, Messaging and AI Provider while
 * rendering rows that pointed at them, and shipped Memory and Billing screens
 * nothing navigated to. None of that failed anything — it was visible only by
 * driving both apps side by side.
 *
 * This is not a diff between web's routes and mobile's. It checks one client
 * against the single registry, which is the declaration of what the product
 * offers. Adding a surface means editing the registry first; this test then
 * says whether mobile has caught up.
 */
const APP_DIR = path.join(__dirname, '..', 'app');

/** Turn an expo-router path into the file that should serve it. */
function routeFileCandidates(route: string): string[] {
  const rel = route.replace(/^\//, '');
  return [
    path.join(APP_DIR, `${rel}.tsx`),
    path.join(APP_DIR, rel, 'index.tsx'),
    path.join(APP_DIR, rel, '_layout.tsx'),
  ];
}

describe('surface parity — mobile', () => {
  const mobileSurfaces = surfacesFor('mobile');

  it('declares at least the primary destinations', () => {
    // Guards against the registry itself being gutted to make this pass.
    expect(mobileSurfaces.length).toBeGreaterThanOrEqual(14);
  });

  it.each(mobileSurfaces.map((s) => [s.id, s.mobile as string]))(
    'implements %s at %s',
    (_id, route) => {
      const found = routeFileCandidates(route).some((candidate) => fs.existsSync(candidate));
      // jest's `expect` takes a single argument, so the explanation rides in the
      // compared value where the failure diff will actually print it.
      const outcome = found
        ? 'implemented'
        : `MISSING — no expo-router file serves ${route}. Build the screen, or set ` +
          'mobile: null in the registry with a "why" if mobile should not have it.';
      expect(outcome).toBe('implemented');
    },
  );

  it('records a reason whenever a platform deliberately lacks a surface', () => {
    // A null without a reason is indistinguishable from an oversight, which is
    // exactly how the original gaps went unnoticed.
    const unexplained = USER_SURFACES.filter(
      (s) => (s.web === null || s.mobile === null) && !s.why,
    );
    expect(unexplained.map((s) => s.id)).toEqual([]);
  });
});
