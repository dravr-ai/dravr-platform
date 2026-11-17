// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for when a sign-in in the athlete's own browser has landed signed in (carnet#1617)
// ABOUTME: Runs the app's rule and the extension's copy (extensions/dravr-signin/signin.js) on one table, so they cannot drift

import { describe, expect, it } from 'vitest';
import type { BrowserSignin } from '@pierre/shared-types';
import { browserSigninLanded, hostOfHttps } from '../src/browserSignin';
// The extension's own copy: plain JavaScript Chrome loads as is, with no bundler to import this package.
import { isSignedIn as extensionLanded, signInRequest } from '../../../extensions/dravr-signin/signin.js';

const GARMIN: BrowserSignin = {
  provider: 'garmin',
  login_url: 'https://sso.garmin.com/portal/sso/en-US/sign-in?clientId=GarminConnect&service=https%3A%2F%2Fconnect.garmin.com%2Fmodern',
  success_patterns: ['/app', '/modern', '/dashboard'],
  cookie_domain: 'garmin.com',
};
const TRAININGPEAKS: BrowserSignin = {
  provider: 'trainingpeaks',
  login_url: 'https://home.trainingpeaks.com/login',
  success_patterns: ['app.trainingpeaks.com', '/Calendar', '/Dashboard'],
  cookie_domain: 'trainingpeaks.com',
};

/** [entry, href, landed] */
const CASES: Array<[BrowserSignin, string, boolean]> = [
  [GARMIN, 'https://connect.garmin.com/app', true],
  [GARMIN, 'https://connect.garmin.com/modern/', true],
  [GARMIN, GARMIN.login_url, false],
  [GARMIN, 'https://evil.test/app', false],
  [GARMIN, 'http://connect.garmin.com/app', false],
  [GARMIN, 'https://evilgarmin.com/app', false],
  [TRAININGPEAKS, 'https://app.trainingpeaks.com/', true],
  [TRAININGPEAKS, 'https://home.trainingpeaks.com/login', false],
];

describe('browserSigninLanded', () => {
  it.each(CASES)('%#: %s lands %s', (entry, href, landed) => {
    expect(browserSigninLanded(href, entry)).toBe(landed);
  });

  it.each(CASES)('%#: the extension reads %s the same way', (entry, href, landed) => {
    const request = signInRequest({
      type: 'signin',
      provider: entry.provider,
      loginUrl: entry.login_url,
      successPatterns: entry.success_patterns,
      cookieDomain: entry.cookie_domain,
    });
    expect(request).not.toBeNull();
    expect(extensionLanded(href, request)).toBe(landed);
  });
});

describe('hostOfHttps', () => {
  it('is the lowercased host of an https URL, its port and path dropped', () => {
    expect(hostOfHttps('https://Connect.Garmin.com:443/app?next=/x')).toBe('connect.garmin.com');
  });

  it('is null for any other scheme or a relative address', () => {
    expect(hostOfHttps('http://connect.garmin.com/app')).toBeNull();
    expect(hostOfHttps('/app')).toBeNull();
  });
});
