// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the takeover socket's address — the API's own origin, ws under http and wss under https, no token in the URL
// ABOUTME: Red if the token reaches the URL (and so request logs), the scheme does not follow the page's, or the path drifts

import { describe, expect, it } from 'vitest';
import type { AxiosInstance } from 'axios';
import {
  SCIOTTE_TAKEOVER_PROTOCOL,
  createOAuthApi,
  sciotteTakeoverProtocols,
  sciotteTakeoverSocketUrl,
} from '../src/domains/oauth';

const TOKEN = 'eyJhbGciOiJIUzI1NiJ9.eyJ0Z3QiOiJzY2lvdHRlX2dhcm1pbiJ9.c2ln-_';
const PROTOCOLS = ['pierre-takeover', `pierre-takeover.${TOKEN}`];

describe('sciotteTakeoverSocketUrl', () => {
  it('opens wss on an https origin, dropping the base path, with no query', () => {
    expect(sciotteTakeoverSocketUrl('https://app.dravr.test/some/path?token=x')).toBe(
      'wss://app.dravr.test/ws/providers/sciotte/takeover',
    );
  });

  it('opens ws on an http origin, keeping its port', () => {
    expect(sciotteTakeoverSocketUrl('http://localhost:5174')).toBe('ws://localhost:5174/ws/providers/sciotte/takeover');
  });

  it('names no URL for a relative base', () => {
    expect(sciotteTakeoverSocketUrl('')).toBeNull();
    expect(sciotteTakeoverSocketUrl('/api')).toBeNull();
  });
});

describe('sciotteTakeoverProtocols', () => {
  it('offers the fixed protocol the server echoes, and the token beside it', () => {
    expect(SCIOTTE_TAKEOVER_PROTOCOL).toBe('pierre-takeover');
    expect(sciotteTakeoverProtocols(TOKEN)).toEqual(PROTOCOLS);
  });
});

describe('oauthApi.sciotteTakeoverSocket', () => {
  it("uses the client's base URL first, and the page's origin only when that is relative", () => {
    const absolute = createOAuthApi({ defaults: { baseURL: 'https://api.dravr.test' } } as unknown as AxiosInstance);
    expect(absolute.sciotteTakeoverSocket(TOKEN, 'http://ignored.test')).toEqual({
      url: 'wss://api.dravr.test/ws/providers/sciotte/takeover',
      protocols: PROTOCOLS,
    });
    const relative = createOAuthApi({ defaults: { baseURL: '' } } as unknown as AxiosInstance);
    expect(relative.sciotteTakeoverSocket(TOKEN, 'https://app.dravr.test')).toEqual({
      url: 'wss://app.dravr.test/ws/providers/sciotte/takeover',
      protocols: PROTOCOLS,
    });
    expect(relative.sciotteTakeoverSocket(TOKEN)).toBeNull();
  });

  it('never puts the token in the URL', () => {
    const api = createOAuthApi({ defaults: { baseURL: 'https://api.dravr.test' } } as unknown as AxiosInstance);
    const socket = api.sciotteTakeoverSocket(TOKEN);
    expect(socket?.url).not.toContain(TOKEN);
    expect(socket?.url).not.toContain('token');
  });
});
