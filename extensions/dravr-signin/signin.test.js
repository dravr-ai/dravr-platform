// ABOUTME: Tests for the extension's pure rules: which sign-ins it accepts, when a tab is signed in, the cookie shape it hands over
// ABOUTME: Runs under `bun test extensions/dravr-signin` with no browser (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, expect, it } from 'bun:test';
import { NOTICE_VERSION, hostUnder, isSignedIn, noticeAccepted, signInRequest, wireCookie } from './signin.js';

const GARMIN = {
  type: 'signin',
  provider: 'garmin',
  loginUrl: 'https://sso.garmin.com/portal/sso/en-US/sign-in?clientId=GarminConnect&service=https%3A%2F%2Fconnect.garmin.com%2Fmodern',
  successPatterns: ['/app', '/modern', '/dashboard'],
  cookieDomain: 'garmin.com',
};

describe('signInRequest', () => {
  it('accepts a provider sign-in under its own domain', () => {
    expect(signInRequest(GARMIN)?.cookieDomain).toBe('garmin.com');
  });

  it('refuses any domain but the four providers', () => {
    expect(signInRequest({ ...GARMIN, cookieDomain: 'google.com', loginUrl: 'https://accounts.google.com/' })).toBeNull();
  });

  it('refuses a sign-in page outside the cookie domain, or over http', () => {
    expect(signInRequest({ ...GARMIN, loginUrl: 'https://evil.test/garmin.com' })).toBeNull();
    expect(signInRequest({ ...GARMIN, loginUrl: 'http://sso.garmin.com/sign-in' })).toBeNull();
  });

  it('refuses a request with no signed-in pattern', () => {
    expect(signInRequest({ ...GARMIN, successPatterns: [] })).toBeNull();
    expect(signInRequest({ ...GARMIN, successPatterns: [''] })).toBeNull();
  });
});

describe('isSignedIn', () => {
  const request = signInRequest(GARMIN);

  it('is the signed-in landing page', () => {
    expect(isSignedIn('https://connect.garmin.com/app', request)).toBe(true);
    expect(isSignedIn('https://connect.garmin.com/modern/', request)).toBe(true);
  });

  it('is not the sign-in page whose query names the landing page', () => {
    expect(isSignedIn(GARMIN.loginUrl, request)).toBe(false);
  });

  it('is not a matching path on another site', () => {
    expect(isSignedIn('https://evil.test/app', request)).toBe(false);
  });

  it('matches a pattern that names the host', () => {
    const tp = signInRequest({
      type: 'signin',
      provider: 'trainingpeaks',
      loginUrl: 'https://home.trainingpeaks.com/login',
      successPatterns: ['app.trainingpeaks.com', '/Calendar'],
      cookieDomain: 'trainingpeaks.com',
    });
    expect(isSignedIn('https://app.trainingpeaks.com/', tp)).toBe(true);
    expect(isSignedIn('https://home.trainingpeaks.com/login', tp)).toBe(false);
  });
});

describe('hostUnder and wireCookie', () => {
  it('reads a subdomain as under its domain, never a lookalike', () => {
    expect(hostUnder('connect.garmin.com', 'garmin.com')).toBe(true);
    expect(hostUnder('evilgarmin.com', 'garmin.com')).toBe(false);
  });

  it('carries the expiry of a persistent cookie and none for a session cookie', () => {
    const base = { name: 'SESSIONID', value: 'v', domain: 'connect.garmin.com', path: '/', secure: true, httpOnly: true };
    expect(wireCookie({ ...base, session: false, expirationDate: 1800000000 })).toEqual({
      name: 'SESSIONID',
      value: 'v',
      domain: 'connect.garmin.com',
      path: '/',
      secure: true,
      http_only: true,
      expires: 1800000000,
    });
    expect('expires' in wireCookie({ ...base, session: true })).toBe(false);
  });
});

describe('noticeAccepted', () => {
  it('holds only for this notice\'s version', () => {
    expect(noticeAccepted({ noticeAccepted: { version: NOTICE_VERSION } })).toBe(true);
    expect(noticeAccepted({ noticeAccepted: { version: NOTICE_VERSION - 1 } })).toBe(false);
    expect(noticeAccepted({})).toBe(false);
    expect(noticeAccepted(undefined)).toBe(false);
  });
});
