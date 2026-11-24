// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The group invite link's path rule — the link a code is shared as, and the code a link's path names
// ABOUTME: One rule for the web app that serves the route and the phone that shares links to it; each passes its own origin

/** The path every group invite link opens, before its code. */
const GROUP_INVITE_PATH = '/groups/join';

/**
 * The shareable link for an invite `code` on `origin` (scheme and host, no
 * trailing slash): the web app's own origin there, the web app's public
 * address on the phone.
 */
export function groupInviteLink(origin: string, code: string): string {
  return `${origin}${GROUP_INVITE_PATH}/${encodeURIComponent(code)}`;
}

/** The invite code a `/groups/join/<code>` path names, or null for any other path. */
export function groupInviteCodeFromPath(pathname: string): string | null {
  const prefix = `${GROUP_INVITE_PATH}/`;
  if (!pathname.startsWith(prefix)) return null;
  const segment = pathname.slice(prefix.length);
  if (segment === '' || segment.includes('/')) return null;
  try {
    const code = decodeURIComponent(segment).trim();
    return code === '' ? null : code;
  } catch (error) {
    // A malformed escape is not a code anyone was given.
    if (error instanceof URIError) return null;
    throw error;
  }
}
