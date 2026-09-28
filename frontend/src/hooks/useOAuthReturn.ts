// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Returns an athlete who left an OAuth authorization to sign in here (Google) back to it
// ABOUTME: Keeps a same-origin /oauth2/authorize path from ?oauth_return= and navigates to it once signed in

import { useEffect } from 'react';

/** Query parameter the hosted OAuth login page links here with. */
const RETURN_PARAM = 'oauth_return';
/** Survives the full-page redirect Google sign-in falls back to. */
const STORAGE_KEY = 'pierre_oauth_return';
/** The one place the return may lead: the authorization it was left from. */
const AUTHORIZE_PREFIX = '/oauth2/authorize?';

/**
 * The pending authorization `value` names, when it is a path on this origin
 * under `/oauth2/authorize` — never another site, which would make this an
 * open redirect.
 */
export function authorizeReturnPath(value: string | null): string | null {
  if (!value || !value.startsWith(AUTHORIZE_PREFIX)) {
    return null;
  }
  try {
    const url = new URL(value, window.location.origin);
    if (url.origin !== window.location.origin || url.pathname !== '/oauth2/authorize') {
      return null;
    }
    return `${url.pathname}${url.search}`;
  } catch {
    return null;
  }
}

function readStored(): string | null {
  try {
    return window.sessionStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeStored(path: string | null): void {
  try {
    if (path) {
      window.sessionStorage.setItem(STORAGE_KEY, path);
    } else {
      window.sessionStorage.removeItem(STORAGE_KEY);
    }
  } catch {
    // Storage blocked: the return is then only good for this page load.
  }
}

/**
 * Send the athlete back to the OAuth authorization they left to sign in.
 *
 * The hosted OAuth login page takes a password only; an athlete who signs in
 * with Google follows its link here with `?oauth_return=`. Once signed in —
 * by whatever method, after a full-page redirect or not — the browser returns
 * to that authorization, which now finds the session and asks for consent.
 */
export function useOAuthReturn(isAuthenticated: boolean): void {
  useEffect(() => {
    const fromQuery = authorizeReturnPath(
      new URLSearchParams(window.location.search).get(RETURN_PARAM),
    );
    if (fromQuery) {
      writeStored(fromQuery);
    }
  }, []);

  useEffect(() => {
    if (!isAuthenticated) {
      return;
    }
    const path = authorizeReturnPath(readStored());
    writeStored(null);
    if (path) {
      window.location.assign(path);
    }
  }, [isAuthenticated]);
}
