// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins a data access reminder's route on both platforms: the privacy pane, opened on the request it names
// ABOUTME: And that a build whose catalogue predates the `privacy` screen opens nothing rather than a wrong screen

import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  MOBILE_DATA_ACCESS_REQUEST_PARAM,
  mobileNotificationTarget,
  resolveNotificationDestination,
  webNotificationRoute,
} from '../src';

const PRIVACY_MOBILE = '/(app)/(tabs)/(settings)/privacy';

describe('a data access reminder (carnet#1659)', () => {
  const reminder = { screen: 'privacy', id: 'req-42', params: { company: 'Acme' } };

  it('names the request it is about', () => {
    expect(resolveNotificationDestination(reminder)).toEqual({
      routes: { web: 'settings/privacy', mobile: PRIVACY_MOBILE },
      dataAccessRequestId: 'req-42',
    });
  });

  it('opens that request in the web privacy pane', () => {
    expect(webNotificationRoute(reminder)).toBe('settings/privacy/data-access/req-42');
    // An id is a path segment, escaped as one.
    expect(webNotificationRoute({ screen: 'privacy', id: 'a/b' })).toBe(
      'settings/privacy/data-access/a%2Fb',
    );
  });

  it('opens that request on the phone privacy screen', () => {
    expect(mobileNotificationTarget(reminder)).toEqual({
      pathname: PRIVACY_MOBILE,
      params: { [MOBILE_DATA_ACCESS_REQUEST_PARAM]: 'req-42' },
    });
  });

  it('opens the pane alone when the payload names no request', () => {
    // The pane is still where every request is listed.
    expect(webNotificationRoute({ screen: 'privacy' })).toBe('settings/privacy');
    expect(webNotificationRoute({ screen: 'privacy', id: 42 })).toBe('settings/privacy');
    expect(mobileNotificationTarget({ screen: 'privacy' })).toEqual({ pathname: PRIVACY_MOBILE });
  });

  it('never reads a privacy id as a conversation', () => {
    expect(resolveNotificationDestination(reminder)?.conversationId).toBeUndefined();
  });
});

describe('a build whose catalogue predates the privacy screen', () => {
  afterEach(() => {
    vi.doUnmock('../src/surface-capabilities.generated');
    vi.resetModules();
  });

  it('resolves the reminder to nothing, so the row reads as information in the centre', async () => {
    // The phone ships the resolver and the generated catalogue in its bundle.
    // A build from before carnet#1659 has the same resolver over a catalogue
    // without `privacy`: the screen is unknown to it, and an unknown screen
    // resolves to null — no navigation, and the notification centre shows the
    // row, whose body names Settings → Privacy & Data, without a link.
    vi.resetModules();
    vi.doMock('../src/surface-capabilities.generated', async (importOriginal) => {
      const actual = await importOriginal<typeof import('../src/surface-capabilities.generated')>();
      const { privacy: _added, ...older } = actual.NOTIFICATION_SCREEN_SURFACES;
      return { ...actual, NOTIFICATION_SCREEN_SURFACES: older };
    });
    const older = await import('../src/notification-routing');
    const reminder = { screen: 'privacy', id: 'req-42' };
    expect(older.resolveNotificationDestination(reminder)).toBeNull();
    expect(older.mobileNotificationTarget(reminder)).toBeNull();
    expect(older.webNotificationRoute(reminder)).toBeNull();
    // The screens it does know still route.
    expect(older.mobileNotificationTarget({ screen: 'connections' })).toEqual({
      pathname: '/(app)/(tabs)/(settings)/connections',
    });
  });
});
