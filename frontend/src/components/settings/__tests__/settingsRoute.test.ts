// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the settings hash subview parser: a pane, or the privacy pane on one data access request
// ABOUTME: Any other trailing shape keeps the pane and opens no request (carnet#1659)

import { describe, expect, it } from 'vitest';
import { webNotificationRoute } from '@pierre/shared-constants';
import { parseSettingsSubview } from '../settingsRoute';

describe('parseSettingsSubview', () => {
  it('reads a pane alone', () => {
    expect(parseSettingsSubview('connections')).toEqual({ pane: 'connections', dataAccessRequest: null });
    expect(parseSettingsSubview('')).toEqual({ pane: '', dataAccessRequest: null });
  });

  it('reads the request a data access reminder opens', () => {
    expect(parseSettingsSubview('privacy/data-access/req-42')).toEqual({
      pane: 'privacy',
      dataAccessRequest: 'req-42',
    });
    expect(parseSettingsSubview('privacy/data-access/a%2Fb')).toEqual({
      pane: 'privacy',
      dataAccessRequest: 'a/b',
    });
  });

  it('parses back the route a reminder writes', () => {
    const route = webNotificationRoute({ screen: 'privacy', id: 'req 7' });
    expect(route).not.toBeNull();
    expect(parseSettingsSubview((route ?? '').replace(/^settings\//, ''))).toEqual({
      pane: 'privacy',
      dataAccessRequest: 'req 7',
    });
  });

  it('keeps the pane and opens nothing for any other shape', () => {
    for (const sub of [
      'privacy/data-access',
      'privacy/data-access/',
      'privacy/other/req-1',
      'privacy/data-access/req-1/extra',
      'privacy/data-access/%E0%A4%A',
    ]) {
      expect(parseSettingsSubview(sub)).toEqual({ pane: 'privacy', dataAccessRequest: null });
    }
    // Only the privacy pane holds requests.
    expect(parseSettingsSubview('connections/data-access/req-1')).toEqual({
      pane: 'connections',
      dataAccessRequest: null,
    });
  });
});
