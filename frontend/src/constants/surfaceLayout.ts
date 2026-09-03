// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Declares which dashboard routes receive the page gutter and which own their own chrome
// ABOUTME: Page padding is a property of the surface, never of the viewer's role

import { webRouteFor } from '@pierre/shared-constants';

/**
 * How a dashboard route fills the content pane.
 *
 * `padded` — the shell owns the page gutter. The surface renders plain content
 * and never thinks about it.
 *
 * `bleed` — the surface is a full-height shell (its own sticky header, its own
 * scroll container, its own internal `px-6`) and must reach the pane edges to
 * be one. It owns every gutter inside it.
 *
 * This used to be a comment on the content pane — "user tabs manage their own
 * layout" — while the gutter itself keyed off `isAdminUser`. Half the surfaces
 * honoured the comment and half assumed a parent gutter that only existed for
 * operators, so Groups, Settings and Data Providers rendered flush against the
 * viewport for every regular user, and looked correct to anyone checking as an
 * admin. A comment cannot fail a build; this table can.
 */
export type SurfaceLayout = 'padded' | 'bleed';

/**
 * Routes that render a full-height shell and pad themselves.
 *
 * Not an exemption list. Each of these is an `h-full flex flex-col` surface
 * with a `TabHeader` and its own scroller, where an outer gutter would
 * double-pad the content and detach the sticky header from the pane edge.
 *
 * Everything absent is `padded`, and that default is the safe direction: a
 * surface nobody classified gets a gutter rather than colliding with the
 * viewport edge. Opting into `bleed` is the deliberate act, and
 * `design-sweep.visual.spec.ts` measures whether the surface then honours it.
 */
const BLEED_ROUTES: ReadonlySet<string> = new Set([
  'chat', // ChatTab — composer pinned to the pane floor, own message scroller
  'discover', // StoreScreen — TabHeader over its own results scroller
  'notifications', // NotificationsPanel — sticky header over its own list scroller
  'settings', // SettingsShell — menu column beside the open section, own scrollers
]);

/** The gutter the shell applies to a `padded` route. */
export const PAGE_GUTTER_CLASS = 'p-4 md:p-6';

/**
 * The same gutter as numbers, so a layout test measures against the value the
 * shell actually applies instead of a second hand-copied constant.
 * Mirrors `PAGE_GUTTER_CLASS`: `p-4` below `md`, `p-6` at or above it.
 */
export const PAGE_GUTTER_PX = { base: 16, md: 24 } as const;

/** Tailwind's `md` breakpoint, in px. */
export const MD_BREAKPOINT_PX = 768;

/** The gutter a `padded` route carries at a given viewport width. */
export function pageGutterPxFor(viewportWidth: number): number {
  return viewportWidth >= MD_BREAKPOINT_PX ? PAGE_GUTTER_PX.md : PAGE_GUTTER_PX.base;
}

/** How the content pane should render `route`. Unknown routes are padded. */
export function layoutForRoute(route: string): SurfaceLayout {
  return BLEED_ROUTES.has(route) ? 'bleed' : 'padded';
}

/**
 * Where the provider-connection pane lives, read from the registry rather than
 * spelled again here.
 *
 * It used to be reachable at two routes — a `#data-providers` tab and the
 * `#settings/connections` section — with the tab's own handler rewriting the
 * hash to the section on first interaction, so the URL a user ended up with
 * depended on which door they came through.
 */
export const CONNECTIONS_ROUTE = webRouteFor('data-providers') ?? 'settings';
