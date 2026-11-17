// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: When a provider sign-in run in the athlete's own browser has landed signed in, for the app's WebView
// ABOUTME: The extension keeps the same rule in its own signin.js; browserSignin.test.ts runs both on one table (carnet#1617)

import type { BrowserSignin } from '@pierre/shared-types';

/** Whether `host` is `domain` or one of its subdomains. */
export function hostUnder(host: string, domain: string): boolean {
  return host === domain || host.endsWith(`.${domain}`);
}

/**
 * The host and path of an absolute `https` URL, never its query: a sign-in
 * page's query can carry the address of the landing page while the sign-in is
 * still on screen. RN's URL polyfill is partial, hence the pattern.
 */
function hostAndPath(href: string): { host: string; path: string } | null {
  const match = /^https:\/\/([^/?#:]+)(?::\d+)?(\/[^?#]*)?/i.exec(href);
  return match ? { host: match[1].toLowerCase(), path: match[2] ?? '/' } : null;
}

/** The lowercased host of an absolute `https` URL, or `null`. */
export function hostOfHttps(href: string): string | null {
  return hostAndPath(href)?.host ?? null;
}

/**
 * Whether the page at `href` is `entry`'s signed-in landing: a page under the
 * provider's cookie domain whose host and path hold one of its signed-in
 * patterns.
 */
export function browserSigninLanded(href: string, entry: BrowserSignin): boolean {
  const at = hostAndPath(href);
  if (!at || !hostUnder(at.host, entry.cookie_domain)) return false;
  const where = `${at.host}${at.path}`;
  return entry.success_patterns.some((pattern) => pattern.length > 0 && where.includes(pattern));
}
