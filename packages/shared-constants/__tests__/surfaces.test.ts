// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the settings destinations to SETTINGS_PANES alone, and each pane to its route on both platforms
// ABOUTME: The ids USER_SURFACES used to repeat them under resolve nowhere, and every server-named screen resolves

import { describe, expect, it } from 'vitest';
import {
  NOTIFICATION_SCREEN_SURFACES,
  SETTINGS_PANES,
  USER_SURFACES,
  settingsPane,
  settingsPaneWebRoute,
  settingsSectionScreen,
  type SettingsPaneId,
} from '../src';
import { destinationRoutes } from '../src/surfaces';

/** Every pane's route on each platform, written out so a moved route fails here. */
const PANE_ROUTES: ReadonlyArray<[SettingsPaneId, string | null, string]> = [
  ['profile', 'settings/profile', '/(app)/(tabs)/(settings)/profile'],
  ['connections', 'settings/connections', '/(app)/(tabs)/(settings)/connections'],
  ['tokens', 'settings/tokens', '/(app)/(tabs)/(settings)/tokens'],
  ['coaching', 'settings/coaching', '/(app)/(tabs)/(settings)/coaching-style'],
  ['messaging', 'settings/messaging', '/(app)/(tabs)/(settings)/messaging'],
  ['notifications', 'settings/notifications', '/(app)/(tabs)/(settings)/notification-preferences'],
  ['memory', 'settings/memory', '/(app)/memory'],
  ['privacy', 'settings/privacy', '/(app)/(tabs)/(settings)/privacy'],
  ['about', 'settings/about', '/(app)/(tabs)/(settings)/about'],
  ['account', 'settings/account', '/(app)/(tabs)/(settings)/account'],
  ['billing', null, '/(app)/billing'],
];

describe('settings destinations', () => {
  it('lists every pane the route table covers, and no other', () => {
    expect(SETTINGS_PANES.map((pane) => pane.id)).toEqual(PANE_ROUTES.map(([id]) => id));
  });

  it.each(PANE_ROUTES)('serves %s at web %s and mobile %s', (id, web, mobile) => {
    expect(settingsPane(id).mobile).toBe(mobile);
    expect(destinationRoutes(id)).toEqual({ web, mobile });
    if (web === null) {
      expect(() => settingsPaneWebRoute(id)).toThrow(`Settings pane "${id}" has no web route`);
    } else {
      expect(settingsPaneWebRoute(id)).toBe(web);
    }
  });

  it('serves connected apps as a screen of the Account pane on the phone', () => {
    expect(settingsSectionScreen('connected-mcp-apps')).toBe('/(app)/(tabs)/(settings)/connected-apps');
    expect(settingsPane('account').holds).toContain('connected-mcp-apps');
    expect(() => settingsSectionScreen('security')).toThrow('section "security"');
  });

  it('declares no settings destination a second time among the top-level surfaces', () => {
    // USER_SURFACES used to carry profile, data-providers, coaching-style,
    // messaging, connected-apps, privacy and memory beside the panes, and both
    // clients navigated through both.
    expect(USER_SURFACES.map((surface) => surface.id)).toEqual([
      'home',
      'chat',
      'discover',
      'notification-center',
      'usage',
      'admin-console',
    ]);
    const settingsRoutes = USER_SURFACES.filter(
      (surface) => surface.web === 'settings' || surface.web?.startsWith('settings/'),
    );
    expect(settingsRoutes).toEqual([]);
  });

  it('resolves none of the ids the settings destinations were duplicated under', () => {
    for (const retired of ['data-providers', 'coaching-style', 'connected-apps']) {
      expect(destinationRoutes(retired)).toBeNull();
    }
  });

  it('shares one id space between the two lists, so an id names one row', () => {
    const paneIds = new Set<string>(SETTINGS_PANES.map((pane) => pane.id));
    expect(USER_SURFACES.filter((surface) => paneIds.has(surface.id)).map((s) => s.id)).toEqual([]);
  });

  it('resolves a top-level surface through the same lookup', () => {
    expect(destinationRoutes('chat')).toEqual({ web: 'chat', mobile: '/(app)/(tabs)/(chat)' });
    expect(destinationRoutes('usage')).toEqual({ web: 'usage', mobile: null });
    expect(destinationRoutes('nowhere')).toBeNull();
  });

  it.each(Object.entries(NOTIFICATION_SCREEN_SURFACES))(
    'resolves the %s notification screen to a destination the server named (%s)',
    (_screen, id) => {
      // The server writes this map. A destination id it names that neither
      // list declares is a notification tap that navigates nowhere.
      expect(destinationRoutes(id)).not.toBeNull();
    },
  );
});
