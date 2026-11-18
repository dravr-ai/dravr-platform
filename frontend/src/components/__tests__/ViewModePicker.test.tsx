// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the admin's sign-in choice between the athlete app and the console (carnet#1631)
// ABOUTME: Each option records its mode; signing out from the picker ends the session it was asked for

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { i18n } from '@pierre/i18n';
import ViewModePicker from '../ViewModePicker';

const chooseViewMode = vi.fn();
const logout = vi.fn();

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: { id: 'admin-1', email: 'jf@dravr.ai', display_name: 'JF', role: 'admin' },
    chooseViewMode,
    logout,
  }),
}));

describe('ViewModePicker', () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    await i18n.changeLanguage('en');
  });

  it('asks the question and offers both modes, athlete first', () => {
    render(<ViewModePicker />);

    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(i18n.t('shell.modePickerTitle'));
    const options = screen.getAllByRole('button').filter((b) => b.dataset.testid?.startsWith('view-mode-'));
    expect(options.map((b) => b.dataset.testid)).toEqual(['view-mode-athlete', 'view-mode-admin']);
  });

  it.each(['athlete', 'admin'] as const)('records the %s mode when chosen', async (mode) => {
    render(<ViewModePicker />);

    await userEvent.click(screen.getByTestId(`view-mode-${mode}`));

    expect(chooseViewMode).toHaveBeenCalledWith(mode);
  });

  it('signs out from the picker', async () => {
    render(<ViewModePicker />);

    await userEvent.click(screen.getByRole('button', { name: i18n.t('shell.navSignOut') }));

    expect(logout).toHaveBeenCalledTimes(1);
  });
});
