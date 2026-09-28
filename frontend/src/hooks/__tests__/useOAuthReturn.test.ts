// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the return to a pending OAuth authorization after signing in through the app (carnet#652)
// ABOUTME: Only a same-origin /oauth2/authorize path is ever followed, across a full-page sign-in redirect

import { renderHook } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { authorizeReturnPath, useOAuthReturn } from '../useOAuthReturn';

const AUTHORIZE = '/oauth2/authorize?client_id=abc&redirect_uri=https%3A%2F%2Fclaude.ai%2Fcb&response_type=code&state=s';

function visit(search: string): void {
  window.history.replaceState(null, '', `/${search}`);
}

describe('authorizeReturnPath', () => {
  it('keeps a same-origin authorize path with its query', () => {
    expect(authorizeReturnPath(AUTHORIZE)).toBe(AUTHORIZE);
  });

  it.each([
    ['another site', 'https://evil.example/oauth2/authorize?x=1'],
    ['a protocol-relative URL', '//evil.example/oauth2/authorize?x=1'],
    ['another path', '/api/admin/users?x=1'],
    ['a path merely starting like it', '/oauth2/authorizeX?x=1'],
    ['nothing', null],
    ['an empty value', ''],
  ])('refuses %s', (_label, value) => {
    expect(authorizeReturnPath(value)).toBeNull();
  });

  it('keeps a traversal-looking query on the authorize endpoint itself', () => {
    // A query cannot move the path: this still lands on /oauth2/authorize.
    expect(authorizeReturnPath('/oauth2/authorize?/../../api')).toBe('/oauth2/authorize?/../../api');
  });
});

describe('useOAuthReturn', () => {
  const assign = vi.fn();

  beforeEach(() => {
    window.sessionStorage.clear();
    assign.mockReset();
    vi.spyOn(window, 'location', 'get').mockReturnValue({
      ...window.location,
      origin: window.location.origin,
      search: window.location.search,
      assign,
    } as Location);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    visit('');
  });

  it('returns a signed-in athlete to the authorization they left', () => {
    vi.restoreAllMocks();
    visit(`?oauth_return=${encodeURIComponent(AUTHORIZE)}`);
    vi.spyOn(window, 'location', 'get').mockReturnValue({
      ...window.location,
      origin: window.location.origin,
      search: `?oauth_return=${encodeURIComponent(AUTHORIZE)}`,
      assign,
    } as Location);

    const { rerender } = renderHook(({ signedIn }) => useOAuthReturn(signedIn), {
      initialProps: { signedIn: false },
    });
    expect(assign).not.toHaveBeenCalled();
    expect(window.sessionStorage.getItem('pierre_oauth_return')).toBe(AUTHORIZE);

    rerender({ signedIn: true });
    expect(assign).toHaveBeenCalledWith(AUTHORIZE);
    expect(window.sessionStorage.getItem('pierre_oauth_return')).toBeNull();
  });

  it('survives the full-page redirect Google sign-in falls back to', () => {
    // The return was stored before the redirect; the page comes back with no query.
    window.sessionStorage.setItem('pierre_oauth_return', AUTHORIZE);
    renderHook(() => useOAuthReturn(true));
    expect(assign).toHaveBeenCalledWith(AUTHORIZE);
  });

  it('never follows a stored value that is not an authorize path', () => {
    window.sessionStorage.setItem('pierre_oauth_return', 'https://evil.example/oauth2/authorize?x=1');
    renderHook(() => useOAuthReturn(true));
    expect(assign).not.toHaveBeenCalled();
    expect(window.sessionStorage.getItem('pierre_oauth_return')).toBeNull();
  });

  it('does nothing for an ordinary sign-in', () => {
    renderHook(() => useOAuthReturn(true));
    expect(assign).not.toHaveBeenCalled();
  });
});
