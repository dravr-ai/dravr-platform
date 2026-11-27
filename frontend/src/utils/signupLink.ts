// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The create-account link's shape — its landing paths and the signup code it carries
// ABOUTME: The code rides the fragment (#code=…) so it never reaches a request URL or a Referer; ?code= is still read

/**
 * The create-account landings. dravr.ai's "Create your account" button points
 * at one of them, so it opens the form it names rather than the sign-in form.
 */
export const REGISTER_PATHS: readonly string[] = ['/register', '/signup'];

/** Whether `pathname` is a create-account landing. */
export function isRegisterLandingPath(pathname: string): boolean {
  return REGISTER_PATHS.includes(pathname);
}

function codeFrom(params: string): string | null {
  const code = new URLSearchParams(params).get('code')?.trim();
  return code ? code : null;
}

/**
 * The signup code a create-account link carries, or `null` (carnet#1128).
 *
 * A link shared as `/signup#code=…` keeps the code out of every request: a
 * fragment is never sent to a server, so it reaches neither a request log nor
 * the Referer of the page's first requests (carnet#1873). The fragment wins
 * over a `?code=` an older link still carries.
 */
export function parseSignupCode(search: string, hash: string): string | null {
  return codeFrom(hash.replace(/^#/, '')) ?? codeFrom(search);
}

