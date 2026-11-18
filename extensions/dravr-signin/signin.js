// ABOUTME: The extension's pure rules: which sign-in it runs, when a tab is signed in, and what each cookie becomes
// ABOUTME: Kept free of chrome.* so signin.test.js runs them under bun test (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/**
 * The domains this extension may read cookies under: the providers Dravr
 * reads through sciotte. The manifest's host permissions name these four and
 * app.dravr.ai, where bridge.js runs; {@link signInRequest} refuses every
 * cookie domain but these four, app.dravr.ai's included.
 */
export const PROVIDER_DOMAINS = Object.freeze(['garmin.com', 'strava.com', 'trainingpeaks.com', 'coros.com']);

/**
 * The storage key, and the version, of the notice the athlete accepts before
 * the first sign-in (notice.html). A new version, for a notice that says
 * something new, asks again.
 */
export const NOTICE_KEY = 'noticeAccepted';
export const NOTICE_VERSION = 1;

/** Whether `stored` (what `storage.local.get(NOTICE_KEY)` answers) holds this notice's acceptance. */
export function noticeAccepted(stored) {
  return stored?.[NOTICE_KEY]?.version === NOTICE_VERSION;
}

/** How long a sign-in tab may stay open before the extension gives up on it. */
export const SIGN_IN_TIMEOUT_MS = 15 * 60 * 1000;

/** Whether `host` is `domain` or one of its subdomains. */
export function hostUnder(host, domain) {
  return host === domain || host.endsWith(`.${domain}`);
}

/**
 * The sign-in a page asked for, when it is one this extension runs: an
 * `https` sign-in page under one of {@link PROVIDER_DOMAINS}, its cookie
 * domain, and at least one signed-in pattern. `null` for anything else.
 */
export function signInRequest(message) {
  if (!message || message.type !== 'signin') return null;
  const { provider, loginUrl, successPatterns, cookieDomain } = message;
  if (typeof provider !== 'string' || typeof cookieDomain !== 'string') return null;
  if (!PROVIDER_DOMAINS.includes(cookieDomain)) return null;
  if (!Array.isArray(successPatterns) || successPatterns.length === 0) return null;
  if (!successPatterns.every((pattern) => typeof pattern === 'string' && pattern.length > 0)) return null;
  let url;
  try {
    url = new URL(loginUrl);
  } catch {
    return null;
  }
  if (url.protocol !== 'https:' || !hostUnder(url.hostname, cookieDomain)) return null;
  return { provider, loginUrl: url.href, successPatterns, cookieDomain };
}

/**
 * Whether the tab at `href` has landed signed in: a page under the
 * provider's domain whose host and path (never its query, which can carry the
 * landing page's address while the sign-in is still on screen) hold one of
 * the provider's signed-in patterns.
 */
export function isSignedIn(href, request) {
  let url;
  try {
    url = new URL(href);
  } catch {
    return false;
  }
  if (url.protocol !== 'https:' || !hostUnder(url.hostname, request.cookieDomain)) return false;
  const where = `${url.hostname}${url.pathname}`;
  return request.successPatterns.some((pattern) => where.includes(pattern));
}

/** A cookie as `chrome.cookies` reads it, in the shape Dravr's sign-in endpoint takes. */
export function wireCookie(cookie) {
  return {
    name: cookie.name,
    value: cookie.value,
    domain: cookie.domain,
    path: cookie.path,
    secure: cookie.secure,
    http_only: cookie.httpOnly,
    ...(cookie.session || cookie.expirationDate === undefined ? {} : { expires: cookie.expirationDate }),
  };
}
