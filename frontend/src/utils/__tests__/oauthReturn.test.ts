// ABOUTME: Tests that a provider connect comes back where it started: its route in this tab, or a tab the chat opened closing
// ABOUTME: Covers the kept route, the result left for the returning surface, and which returns keep the callback page
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  OAUTH_RESULT_KEY,
  historyLoginAfterGrant,
  returnToConnectStart,
  settleOAuthReturn,
  startOAuthInNewTab,
  startOAuthInThisTab,
  type OAuthCallbackParams,
} from '../oauthReturn';
import type { ExtendedProviderStatus } from '@pierre/shared-types';
import { isSessionRouteState } from '../sessionRoute';

const RETURN_ROUTE_KEY = 'pierre_oauth_return';
const TOMBSTONE = { tombstoneId: 'tomb-1', deletedAt: '2026-10-20T08:00:00+00:00' };

function storedResult(): Record<string, unknown> {
  return JSON.parse(localStorage.getItem(OAUTH_RESULT_KEY) ?? '{}');
}

let close: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  // jsdom's close() tears the test window down; a browser that refuses it
  // (a tab no script opened) leaves the page running, which is what follows.
  close = vi.spyOn(window, 'close').mockImplementation(() => undefined);
});

afterEach(() => {
  window.history.replaceState(null, '', '/');
  close.mockRestore();
});

describe('startOAuthInThisTab', () => {
  it('keeps the dashboard route without its #, then goes to the launch URL in this tab', () => {
    const assign = vi.fn();
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    const original = window.location;
    Object.defineProperty(window, 'location', {
      configurable: true,
      value: { ...original, hash: '#settings/connections', assign },
    });
    try {
      startOAuthInThisTab('/api/oauth/authorize/garmin');

      expect(sessionStorage.getItem(RETURN_ROUTE_KEY)).toBe('settings/connections');
      expect(assign).toHaveBeenCalledWith('/api/oauth/authorize/garmin');
      // The connect never opens the provider in a second tab.
      expect(open).not.toHaveBeenCalled();
    } finally {
      Object.defineProperty(window, 'location', { configurable: true, value: original });
      open.mockRestore();
    }
  });
});

describe('startOAuthInNewTab', () => {
  it('opens the connect in a script-opened tab and cuts its way back into this one', () => {
    const tab = { opener: window as unknown } as Window;
    const open = vi.spyOn(window, 'open').mockReturnValue(tab);
    try {
      startOAuthInNewTab('https://connect.garmin.com/oauth2Confirm?state=s');

      expect(open).toHaveBeenCalledWith('https://connect.garmin.com/oauth2Confirm?state=s', '_blank');
      expect(tab.opener).toBeNull();
      // This tab keeps no route: the new tab closes itself on the return.
      expect(sessionStorage.getItem(RETURN_ROUTE_KEY)).toBeNull();
    } finally {
      open.mockRestore();
    }
  });
});

describe('settleOAuthReturn', () => {
  it('has nothing to settle on an ordinary load', () => {
    expect(settleOAuthReturn(null)).toBeNull();
    expect(localStorage.getItem(OAUTH_RESULT_KEY)).toBeNull();
  });

  it('leaves a grant for the returning surface and puts the starting route back', () => {
    sessionStorage.setItem(RETURN_ROUTE_KEY, 'coaches');
    window.history.replaceState(null, '', '/?provider=strava&success=true');

    expect(settleOAuthReturn({ provider: 'strava', success: true })).toBeNull();

    expect(`${window.location.pathname}${window.location.search}${window.location.hash}`).toBe('/#coaches');
    // Written as this session's own route, not a followed link.
    expect(isSessionRouteState(window.history.state)).toBe(true);
    expect(sessionStorage.getItem(RETURN_ROUTE_KEY)).toBeNull();
    const stored = storedResult();
    expect(stored.type).toBe('oauth_completed');
    expect(stored.provider).toBe('strava');
    expect(stored.success).toBe(true);
    expect(stored.hasError).toBe(false);
    expect(stored.tombstoned).toBe(false);
    expect(typeof stored.timestamp).toBe('number');
  });

  it('sends a refused Garmin grant straight back too, where the card offers the login', () => {
    sessionStorage.setItem(RETURN_ROUTE_KEY, 'home');
    window.history.replaceState(null, '', '/?provider=garmin&success=false');

    expect(settleOAuthReturn({ provider: 'garmin', success: false, error: 'access_denied' })).toBeNull();

    expect(window.location.search).toBe('');
    expect(window.location.hash).toBe('#home');
    const stored = storedResult();
    expect(stored.provider).toBe('garmin');
    expect(stored.success).toBe(false);
    expect(stored.hasError).toBe(true);
  });

  it('keeps the callback page for any other refusal and leaves the address bar alone', () => {
    sessionStorage.setItem(RETURN_ROUTE_KEY, 'home');
    window.history.replaceState(null, '', '/?provider=whoop&success=false');
    const params: OAuthCallbackParams = { provider: 'whoop', success: false, error: 'access_denied' };

    expect(settleOAuthReturn(params)).toBe(params);

    expect(window.location.search).toBe('?provider=whoop&success=false');
    // The page's Continue button still needs the route to go back to.
    expect(sessionStorage.getItem(RETURN_ROUTE_KEY)).toBe('home');
    expect(storedResult().success).toBe(false);
  });

  it('closes a tab the chat opened once its grant is settled, leaving the result for the thread', () => {
    window.history.replaceState(null, '', '/?provider=garmin&success=true');

    expect(settleOAuthReturn({ provider: 'garmin', success: true })).toBeNull();

    expect(close).toHaveBeenCalledTimes(1);
    expect(storedResult().success).toBe(true);
    // Refused by the browser, the tab falls back to the app root.
    expect(`${window.location.pathname}${window.location.search}${window.location.hash}`).toBe('/');
  });

  it('keeps the callback page for a refused Garmin grant in a tab the chat opened, where no card answers it', () => {
    window.history.replaceState(null, '', '/?provider=garmin&success=false');
    const params: OAuthCallbackParams = { provider: 'garmin', success: false, error: 'access_denied' };

    expect(settleOAuthReturn(params)).toBe(params);

    expect(close).not.toHaveBeenCalled();
    expect(window.location.search).toBe('?provider=garmin&success=false');
  });

  it('keeps the callback page for a tombstone, even on Strava, and marks the result', () => {
    window.history.replaceState(null, '', '/?provider=strava&success=false');
    const params: OAuthCallbackParams = {
      provider: 'strava',
      success: false,
      error: 'import_tombstoned',
      tombstone: TOMBSTONE,
    };

    expect(settleOAuthReturn(params)).toBe(params);

    expect(window.location.search).toBe('?provider=strava&success=false');
    const stored = storedResult();
    expect(stored.tombstoned).toBe(true);
    expect(stored.hasError).toBe(true);
    // The raw error is never kept, only that one occurred.
    expect(JSON.stringify(stored)).not.toContain('import_tombstoned');
  });
});

describe('returnToConnectStart', () => {
  it('puts the kept route back as a session route and forgets it', () => {
    sessionStorage.setItem(RETURN_ROUTE_KEY, 'settings/connections');
    window.history.replaceState(null, '', '/?provider=whoop&success=false');

    returnToConnectStart();

    expect(close).not.toHaveBeenCalled();
    expect(`${window.location.pathname}${window.location.search}${window.location.hash}`).toBe(
      '/#settings/connections',
    );
    expect(isSessionRouteState(window.history.state)).toBe(true);
    expect(sessionStorage.getItem(RETURN_ROUTE_KEY)).toBeNull();
  });

  it('closes a tab the connect did not start in, else leaves the app root for the role default', () => {
    window.history.replaceState(null, '', '/?provider=whoop&success=false');

    returnToConnectStart();

    expect(close).toHaveBeenCalledTimes(1);
    expect(`${window.location.pathname}${window.location.search}${window.location.hash}`).toBe('/');
    expect(isSessionRouteState(window.history.state)).toBe(false);
  });
});

describe('historyLoginAfterGrant', () => {
  // The Garmin card as the server serves it after a Garmin grant.
  const garmin = (awaiting: boolean) =>
    ({
      provider: 'sciotte_garmin',
      connected: true,
      connected_backend: 'garmin',
      oauth_awaiting_activities: awaiting,
    }) as ExtendedProviderStatus;

  it('leads a Garmin grant with nothing pushed yet to the Garmin card', () => {
    const card = garmin(true);
    expect(historyLoginAfterGrant({ provider: 'garmin', success: true }, [card])).toBe(card);
  });

  it('leads nowhere once the grant holds activities, on a refusal, or for another grant', () => {
    expect(historyLoginAfterGrant({ provider: 'garmin', success: true }, [garmin(false)])).toBeNull();
    expect(historyLoginAfterGrant({ provider: 'garmin', success: false }, [garmin(true)])).toBeNull();
    expect(historyLoginAfterGrant({ provider: 'whoop', success: true }, [garmin(true)])).toBeNull();
  });
});
