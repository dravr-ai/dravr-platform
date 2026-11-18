// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Carries the code a /groups/join/<code> link opened in this tab until the server holds it (carnet#1648)
// ABOUTME: Kept in sessionStorage across sign-in redirects and reloads; dropped only on a 2xx, or a 400/404 saying it is dead

/** The sessionStorage key the carried code survives the tab's reloads under. */
const GROUP_INVITE_CARRY_KEY = 'dravr.pending_invite_code';

const INVITE_PATH = /^\/groups\/join\/([^/]+)$/;

/**
 * The code this page carries when the browser refuses storage (a private
 * window): the invite then lasts as long as the page. Read only when storage
 * throws, so a working storage is the one record.
 */
let pageCarry: string | null = null;

/** The invite code a `/groups/join/<code>` path names, or null for any other path. */
export function groupInviteCodeFromPath(pathname: string): string | null {
  const match = pathname.match(INVITE_PATH);
  if (!match) return null;
  try {
    const code = decodeURIComponent(match[1]).trim();
    return code === '' ? null : code;
  } catch (error) {
    // A malformed escape is not a code anyone was given.
    if (error instanceof URIError) return null;
    throw error;
  }
}

/** The invite code this tab carries, or null. */
export function readCarriedGroupInvite(): string | null {
  try {
    return window.sessionStorage.getItem(GROUP_INVITE_CARRY_KEY);
  } catch {
    return pageCarry;
  }
}

/**
 * Forget the carried code once the server has answered for it: it holds the
 * invite now, or said the code is dead. A newer link's code stays carried.
 */
export function dropCarriedGroupInvite(code: string): void {
  if (pageCarry === code) pageCarry = null;
  try {
    if (window.sessionStorage.getItem(GROUP_INVITE_CARRY_KEY) === code) {
      window.sessionStorage.removeItem(GROUP_INVITE_CARRY_KEY);
    }
  } catch {
    // Storage refused: the page's own copy above was the only one.
  }
}

/**
 * Read a `/groups/join/<code>` link this page opened on: carry its code for the
 * tab, and put `/` in the address bar so a reload does not reopen the link.
 * Call it once, before the app mounts.
 */
export function captureGroupInviteLink(): void {
  const code = groupInviteCodeFromPath(window.location.pathname);
  if (code === null) return;
  pageCarry = code;
  try {
    window.sessionStorage.setItem(GROUP_INVITE_CARRY_KEY, code);
  } catch {
    // Storage refused: the page's own copy carries it until the page goes.
  }
  window.history.replaceState({}, document.title, '/');
}
