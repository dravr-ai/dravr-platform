// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the settings menu — one row per section with its hint, the active row marked, sign out last
// ABOUTME: Pins the accessible names the e2e suite and the shell rely on

import { describe, expect, it, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SettingsMenu from '../SettingsMenu';
import { SETTINGS_TABS } from '../settingsTabs';

describe('SettingsMenu', () => {
  it('lists every section with its name and hint, marks the open one, and signs out last', async () => {
    const onSelect = vi.fn();
    const onSignOut = vi.fn();
    render(
      <SettingsMenu
        tabs={SETTINGS_TABS}
        activeTab="memory"
        onSelect={onSelect}
        displayName="Maya Tremblay"
        email="maya@example.com"
        onSignOut={onSignOut}
      />,
    );

    const nav = screen.getByRole('navigation', { name: 'Settings tabs' });
    expect(within(nav).getByText('Maya Tremblay')).toBeInTheDocument();
    expect(within(nav).getByText('maya@example.com')).toBeInTheDocument();

    const rows = within(nav).getAllByRole('listitem');
    expect(rows).toHaveLength(SETTINGS_TABS.length);
    expect(within(nav).getByText('Data providers')).toBeInTheDocument();
    expect(within(nav).getByText('Strava, Garmin and the data your agent reads')).toBeInTheDocument();
    // The row is named by its label alone; the hint is its description, so a
    // name lookup for one section never resolves to another whose hint mentions it.
    const connections = screen.getByTestId('settings-menu-connections');
    expect(connections).toHaveAccessibleName('Data providers');
    expect(connections).toHaveAccessibleDescription('Strava, Garmin and the data your agent reads');
    expect(within(nav).getByRole('button', { name: 'About' })).toBe(screen.getByTestId('settings-menu-about'));

    const memory = screen.getByTestId('settings-menu-memory');
    expect(memory).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTestId('settings-menu-profile')).not.toHaveAttribute('aria-current');

    await userEvent.click(screen.getByTestId('settings-menu-connections'));
    expect(onSelect).toHaveBeenCalledWith('connections');

    await userEvent.click(within(nav).getByRole('button', { name: 'Sign out' }));
    expect(onSignOut).toHaveBeenCalledTimes(1);
  });
});
