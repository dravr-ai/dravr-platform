// ABOUTME: Asserts web implements every surface the shared registry declares for it
// ABOUTME: Checks the real Dashboard source and chat renderer, so a declared-but-unbuilt route fails here

import fs from 'fs';
import path from 'path';
import { describe, it, expect } from 'vitest';
import { SETTINGS_PANES, SURFACE_CAPABILITIES, USER_SURFACES, surfacesFor } from '@pierre/shared-constants';
import { defaultI18nConfig } from '@pierre/i18n';

/**
 * Mobile has had this test since the registry existed. Web never did — a grep
 * for `USER_SURFACES` found consumers in the package barrel and the mobile test
 * and nowhere else, so the registry's `web` column was enforced by nobody and a
 * route could be declared here, linked from a notification, and simply not
 * exist.
 *
 * Like the mobile one, this is not a diff between the two clients. It checks
 * one client against the single registry: adding a surface means editing the
 * registry first, and this says whether web has caught up.
 */
const SRC = path.join(__dirname, '..');
const DASHBOARD = path.join(SRC, 'components', 'Dashboard.tsx');
const CHAT_RENDERER = path.join(SRC, 'components', 'chat', 'MessageItem.tsx');

/** The Dashboard is a hash router over tab ids; its source is the route table. */
const dashboardSource = fs.readFileSync(DASHBOARD, 'utf8');
const chatRendererSource = fs.readFileSync(CHAT_RENDERER, 'utf8');


/**
 * A web route exists when the Dashboard either declares it as a sidebar tab or
 * renders something for it. Both count: settings sub-surfaces are reachable
 * through the gear rather than the sidebar, so requiring a `TabDefinition`
 * would fail six surfaces that genuinely work.
 */
function dashboardServes(route: string): boolean {
  const [tab, section] = route.split('/');
  const servesTab =
    dashboardSource.includes(`id: '${tab}'`) || dashboardSource.includes(`activeTab === '${tab}'`);
  if (section === undefined) return servesTab;
  // A `settings/<section>` route is served when the Dashboard serves `settings`
  // and the pane registry declares that section — the shape the connections
  // pane uses now that it is no longer also a top-level tab of its own.
  return servesTab && SETTINGS_PANES.some((pane) => pane.id === section);
}

describe('surface parity — web', () => {
  const webSurfaces = surfacesFor('web');

  it('declares at least the primary destinations', () => {
    // Guards against the registry itself being gutted to make this pass. The
    // floor dropped from 14 when the Chat-First Cutover retired Insights and
    // folded the Coach tab into Discover, and again to 12 when group
    // management moved into the group's own chat thread — each one surface
    // fewer by decision.
    expect(webSurfaces.length).toBeGreaterThanOrEqual(12);
  });

  it('no longer declares the retired Groups surface', () => {
    // Group management lives in the group's chat thread (`/group …` and the
    // header's Group info); a registry row pointing at a Groups tab would send
    // a deep link to a destination that no longer exists.
    expect(USER_SURFACES.find((s) => s.id === 'groups')).toBeUndefined();
  });

  it.each(webSurfaces.map((s) => [s.id, s.web as string]))(
    'implements %s at %s',
    (_id, route) => {
      // vitest's `expect` takes a single argument, so the explanation rides in
      // the compared value where the failure diff will actually print it.
      const outcome = dashboardServes(route)
        ? 'implemented'
        : `MISSING — Dashboard.tsx serves no '${route}' tab. Build the surface, or set ` +
          'web: null in the registry with a "why" if web should not have it.';
      expect(outcome).toBe('implemented');
    },
  );

  it('renders every reply block the chat surface declares', () => {
    // The registry's `blocks` column comes from the server's own capability
    // table. A kind the server will send this surface and the renderer has no
    // arm for is a block the athlete never sees — silently, because an
    // unmatched switch arm renders nothing and throws nothing.
    const chat = USER_SURFACES.find((s) => s.id === 'chat');
    expect(chat?.blocks.length).toBeGreaterThanOrEqual(8);
    const unhandled = (chat?.blocks ?? []).filter(
      (kind) => !chatRendererSource.includes(`case '${kind}':`),
    );
    expect(unhandled).toEqual([]);
  });

  it('reads the same capability row as mobile', () => {
    // The registry publishes one `blocks` column for the chat surface. That is
    // only honest while both in-app rows resolve the same capabilities, so it
    // is asserted rather than assumed.
    expect(SURFACE_CAPABILITIES.web_chat.blocks).toEqual(SURFACE_CAPABILITIES.mobile_chat.blocks);
    expect(SURFACE_CAPABILITIES.web_chat.max_reply_chars).toBe(
      SURFACE_CAPABILITIES.mobile_chat.max_reply_chars,
    );
    expect(SURFACE_CAPABILITIES.web_chat.progressive).toBe('delta_channel');
  });

  it('pins every web nav label to a surface the sidebar actually offers', () => {
    // The design sweep walks these labels; a label no sidebar button carries
    // makes the sweep skip a surface and still report success.
    //
    // The sidebar renders its labels through the corpus, so this checks the
    // same two links the sweep depends on: the route exists in the sidebar,
    // and the registry's English `webNav` is exactly what `nav.*` renders in
    // English. It used to match on a `name: '<label>'` literal in the source,
    // which the translation removed — and which would have gone on passing for
    // the admin tabs that still carry literals, quietly covering less than it
    // claimed.
    const bundle = defaultI18nConfig.resources.en.translation as { nav: Record<string, string> };
    const navLabels = new Set(Object.values(bundle.nav));

    const missing = USER_SURFACES.filter((s) => {
      if (s.webNav === null) {
        return false;
      }
      return !navLabels.has(s.webNav) || !dashboardSource.includes(`id: '${s.web}'`);
    });
    expect(missing.map((s) => s.id)).toEqual([]);
  });
});
