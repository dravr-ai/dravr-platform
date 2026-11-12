// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the takeover socket's URL — the API's own origin, ws under http and wss under https, the token as a query parameter
// ABOUTME: Red if the scheme does not follow the page's, the path drifts from the server's route, or the token is not encoded

import { describe, expect, it } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createOAuthApi, sciotteTakeoverSocketUrl } from '../src/domains/oauth';

const TOKEN = 'eyJhbGciOiJIUzI1NiJ9.eyJ0Z3QiOiJzY2lvdHRlX2dhcm1pbiJ9.sig+/=';
const ENCODED = encodeURIComponent(TOKEN);

describe('sciotteTakeoverSocketUrl', () => {
  it('opens wss on an https origin, dropping the base path', () => {
    expect(sciotteTakeoverSocketUrl('https://app.dravr.test/some/path', TOKEN)).toBe(
      `wss://app.dravr.test/ws/providers/sciotte/takeover?token=${ENCODED}`,
    );
  });

  it('opens ws on an http origin, keeping its port', () => {
    expect(sciotteTakeoverSocketUrl('http://localhost:5174', TOKEN)).toBe(
      `ws://localhost:5174/ws/providers/sciotte/takeover?token=${ENCODED}`,
    );
  });

  it('names no URL for a relative base', () => {
    expect(sciotteTakeoverSocketUrl('', TOKEN)).toBeNull();
    expect(sciotteTakeoverSocketUrl('/api', TOKEN)).toBeNull();
  });

  it("uses the client's base URL first, and the page's origin only when that is relative", () => {
    const absolute = createOAuthApi({ defaults: { baseURL: 'https://api.dravr.test' } } as unknown as AxiosInstance);
    expect(absolute.sciotteTakeoverSocketUrl(TOKEN, 'http://ignored.test')).toBe(
      `wss://api.dravr.test/ws/providers/sciotte/takeover?token=${ENCODED}`,
    );
    const relative = createOAuthApi({ defaults: { baseURL: '' } } as unknown as AxiosInstance);
    expect(relative.sciotteTakeoverSocketUrl(TOKEN, 'https://app.dravr.test')).toBe(
      `wss://app.dravr.test/ws/providers/sciotte/takeover?token=${ENCODED}`,
    );
    expect(relative.sciotteTakeoverSocketUrl(TOKEN)).toBeNull();
  });
});
