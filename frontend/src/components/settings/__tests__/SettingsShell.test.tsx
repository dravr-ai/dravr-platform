// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What each viewer's settings list holds: an operator's profile and account, never an athlete's own panes
// ABOUTME: The two token panes follow one flag together, in the athlete menu and the operator strip alike

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import SettingsShell from '../SettingsShell';
import { visibleSettingsTabs } from '../visibleSettingsTabs';

const flags = vi.hoisted(() => ({ api_tokens: true }));

vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: { id: 'u-1', email: 'maya@acme.com', display_name: 'Maya', role: 'user' },
    logout: vi.fn(),
  }),
}));

vi.mock('../../../hooks/useFeatureFlags', () => ({
  useFeatureFlags: () => ({ flags }),
  FEATURE_KEYS: { apiTokens: 'api_tokens' },
}));

// The menu is what is under test; the wide layout keeps it on screen beside a pane.
vi.mock('../../../hooks/useBreakpoint', () => ({ useIsDesktop: () => true }));
vi.mock('../../UserSettings', () => ({ default: () => null }));

function menuRows() {
  render(<SettingsShell tab="profile" onSelect={vi.fn()} onBack={vi.fn()} />);
  return screen
    .getAllByTestId(/^settings-menu-/)
    .map((row) => row.getAttribute('data-testid')?.replace('settings-menu-', ''));
}

describe('settings an athlete is offered', () => {
  beforeEach(() => {
    flags.api_tokens = true;
  });

  it('lists the token panes and the panes hidden from operators', () => {
    expect(menuRows()).toEqual(
      expect.arrayContaining(['tokens', 'api-keys', 'coaching', 'notifications', 'memory', 'privacy']),
    );
  });

  it('drops both token panes when the flag is off', () => {
    flags.api_tokens = false;
    const rows = menuRows();
    expect(rows).not.toContain('tokens');
    expect(rows).not.toContain('api-keys');
    expect(rows).toContain('memory');
  });
});

describe('settings an operator is offered', () => {
  const ids = (apiTokens: boolean) =>
    visibleSettingsTabs(true, apiTokens)
      .map((tab) => tab.id)
      .sort();

  it('leaves their profile, account and tokens', () => {
    expect(ids(true)).toEqual(['account', 'api-keys', 'profile', 'tokens']);
  });

  it('drops both token panes when the flag is off', () => {
    expect(ids(false)).toEqual(['account', 'profile']);
  });
});
