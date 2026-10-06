// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tells a dashboard route the app wrote from a deep link the athlete followed
// ABOUTME: A route written during a session ends with it, so the next sign-in lands on the role default

/**
 * The `history.state` the dashboard writes beside every `#tab[/subview]` it
 * puts in the address bar. The browser keeps an entry's state across a reload
 * and a restored tab, but a followed link or a typed URL is a fresh entry whose
 * state is `null`. That difference is the only thing that separates "the page
 * the athlete was on when the session died" from "the page a link sent them to":
 * both arrive as the same hash on the login screen.
 */
const SESSION_ROUTE_STATE = { pierreSessionRoute: true } as const;

/** Whether a `history.state` value is one the dashboard wrote for its own route. */
export function isSessionRouteState(state: unknown): boolean {
  return (
    typeof state === 'object' &&
    state !== null &&
    (state as { pierreSessionRoute?: unknown }).pierreSessionRoute === true
  );
}

function currentHashRoute(): string {
  return window.location.hash.replace(/^#/, '');
}

/**
 * The route a followed link asked for when this page loaded, until a session
 * has shown it. Read once, at module evaluation — before the first render,
 * whose route sync marks the entry and so erases the evidence.
 */
let pendingDeepLink: string | null =
  typeof window !== 'undefined' && currentHashRoute() !== '' && !isSessionRouteState(window.history.state)
    ? currentHashRoute()
    : null;

/**
 * Write the dashboard's route into the address bar, marked as a session route.
 * `push` adds a Back step; `replace` rewrites the current entry.
 */
export function writeSessionRoute(route: string, mode: 'push' | 'replace'): void {
  if (mode === 'push') {
    window.history.pushState(SESSION_ROUTE_STATE, '', `#${route}`);
  } else {
    window.history.replaceState(SESSION_ROUTE_STATE, '', `#${route}`);
  }
}

/**
 * Mark the current entry as a session route when something other than the
 * dashboard's own sync put the hash there — an in-app `location.hash =`
 * assignment, an anchor — so a reload of it still reads as the session's page.
 */
export function markCurrentSessionRoute(): void {
  if (!isSessionRouteState(window.history.state)) {
    window.history.replaceState(SESSION_ROUTE_STATE, '', window.location.href);
  }
}

/**
 * The followed link still waiting for a session, if any. The hosted sign-in is
 * a full-page round trip that returns to `/auth/callback` with no hash, so the
 * sign-in keeps this beside its PKCE verifier and puts it back on return.
 */
export function peekDeepLink(): string | null {
  return pendingDeepLink;
}

/**
 * Put a followed link back in the address bar after the hosted sign-in's round
 * trip, as the followed link it was (unmarked, so the dashboard opens it), and
 * hold it as pending until the session that is being established shows it.
 * With no link, the address bar is left at the app root and the dashboard
 * resolves the role default.
 */
export function restoreDeepLink(route: string | null): void {
  pendingDeepLink = route;
  window.history.replaceState(null, '', route ? `/#${route}` : '/');
}

/**
 * A session has been established and the dashboard it opens has read the
 * followed link: it is spent, and a later sign-out must not replay it.
 */
export function consumeDeepLink(): void {
  pendingDeepLink = null;
}

/**
 * The session is over, so the route it was on is over too. Rewrite the address
 * bar to the followed link that has not been shown yet, or to no route at all —
 * which the dashboard resolves to the role default (Home for an athlete) on the
 * next sign-in, whichever way that sign-in happens.
 */
export function endSessionRoute(): void {
  if (typeof window === 'undefined') return;
  const target = pendingDeepLink ? `#${pendingDeepLink}` : '';
  // Always rewrite, even onto the same hash: the optimistic render of a cached
  // user may already have marked the followed link as a session route, and a
  // reload of the login screen must still read it as the link it is.
  window.history.replaceState(null, '', `${window.location.pathname}${window.location.search}${target}`);
}
