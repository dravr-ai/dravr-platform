// ABOUTME: Asserts mobile implements every surface the shared registry declares for it
// ABOUTME: Checks real expo-router files on disk, so a declared-but-unbuilt screen fails here

import fs from 'fs';
import path from 'path';
import { SURFACE_CAPABILITIES, USER_SURFACES, surfacesFor } from '@pierre/shared-constants';

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
const CHAT_RENDERER = path.join(__dirname, '..', 'src', 'screens', 'chat', 'MessageList.tsx');

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

  it('renders every reply block the chat surface declares', () => {
    // The registry's `blocks` column comes from the server's own capability
    // table. A kind the server will send this surface and the renderer has no
    // arm for is a block the athlete never sees — silently, because an
    // unmatched switch arm renders nothing and throws nothing.
    const chat = USER_SURFACES.find((s) => s.id === 'chat');
    const renderer = fs.readFileSync(CHAT_RENDERER, 'utf8');
    expect(chat?.blocks.length).toBeGreaterThanOrEqual(8);
    const unhandled = (chat?.blocks ?? []).filter(
      (kind) => !renderer.includes(`case '${kind}':`),
    );
    expect(unhandled).toEqual([]);
  });

  it('reads the same capability row as web', () => {
    // The registry publishes one `blocks` column for the chat surface. That is
    // only honest while both in-app rows resolve the same capabilities, so it
    // is asserted rather than assumed.
    expect(SURFACE_CAPABILITIES.mobile_chat.blocks).toEqual(SURFACE_CAPABILITIES.web_chat.blocks);
    expect(SURFACE_CAPABILITIES.mobile_chat.max_reply_chars).toBe(
      SURFACE_CAPABILITIES.web_chat.max_reply_chars,
    );
    expect(SURFACE_CAPABILITIES.mobile_chat.progressive).toBe('delta_channel');
  });

  it('records a reason whenever a platform deliberately lacks a surface', () => {
    // A null without a reason is indistinguishable from an oversight, which is
    // exactly how the original gaps went unnoticed.
    const unexplained = USER_SURFACES.filter(
      (s) => (s.web === null || s.mobile === null) && !s.why,
    );
    expect(unexplained.map((s) => s.id)).toEqual([]);
  });
});
