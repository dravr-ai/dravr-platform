// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the query string an OAuth start carries on the web launch route and the mobile init
// ABOUTME: Red if a flag's wire name drifts from the server's (`tos_consent`, `sign_in_blocked`) or an unset flag rides along

import { describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createOAuthApi } from '../src/domains/oauth';

describe('the OAuth start query (carnet#1667)', () => {
  const web = createOAuthApi({ defaults: { baseURL: 'https://api.dravr.test/' } } as unknown as AxiosInstance);

  it('carries no query on a plain web launch', () => {
    expect(web.authorizeUrl('garmin')).toBe('https://api.dravr.test/api/oauth/authorize/garmin');
  });

  it('names a fallback from a blocked sign-in, and the notice the athlete accepted, by the names the server reads', () => {
    expect(web.authorizeUrl('strava', { signInBlocked: true })).toBe(
      'https://api.dravr.test/api/oauth/authorize/strava?sign_in_blocked=true',
    );
    expect(web.authorizeUrl('whoop', { tosConsent: true, signInBlocked: false })).toBe(
      'https://api.dravr.test/api/oauth/authorize/whoop?tos_consent=true',
    );
  });

  it('adds the flags after the mobile return address', async () => {
    const get = vi.fn().mockResolvedValue({ data: { authorization_url: 'https://connect.garmin.test/', provider: 'garmin' } });
    const mobile = createOAuthApi({ get, defaults: {} } as unknown as AxiosInstance);

    await mobile.initMobileOAuth('garmin', 'dravr://oauth-callback', { tosConsent: false, signInBlocked: true });
    expect(get).toHaveBeenLastCalledWith(
      '/api/oauth/mobile/init/garmin?redirect_uri=dravr%3A%2F%2Foauth-callback&sign_in_blocked=true',
    );

    await mobile.initMobileOAuth('garmin');
    expect(get).toHaveBeenLastCalledWith('/api/oauth/mobile/init/garmin');
  });
});
