// ABOUTME: Pins the settings root's row grammar — a pane row is its name and hint with a chevron, no glyph square and no card
// ABOUTME: And the quiet sign-out after the list: it asks before signing out, and the confirm button is what signs out

import React from 'react';
import { Alert } from 'react-native';
import { fireEvent, render, within } from '@testing-library/react-native';
import type { User } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import { settingsPanesFor } from '@pierre/shared-constants';

const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, back: jest.fn() }),
  useFocusEffect: () => undefined,
}));

// The glyph as a text node carrying its name, so a spec can list every icon
// the row drew and tell a chevron from anything else.
jest.mock('@expo/vector-icons', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    Feather: ({ name, color, size }: { name: string; color: string; size: number }) =>
      React.createElement(Text, { testID: `icon-${name}`, style: { color, fontSize: size } }, name),
  };
});

jest.mock('../src/services/api', () => ({
  userApi: {
    getMcpTokens: jest.fn().mockResolvedValue({ tokens: [] }),
  },
  oauthApi: {
    getProvidersStatus: jest.fn().mockResolvedValue({ providers: [] }),
  },
}));

const mockLogout = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    user: {
      id: 'user-1',
      email: 'mobiletest@pierre.dev',
      display_name: 'Mobile Test User',
      is_admin: false,
      role: 'user',
      user_status: 'active',
    } as Partial<User>,
    logout: mockLogout,
    isAuthenticated: true,
    updateUser: jest.fn(),
  }),
}));

jest.mock('../src/hooks/useFeatureFlags', () => ({
  useFeatureFlags: () => ({ flags: { api_tokens: false, billing_header: false }, known: [], isLoading: false, isError: false }),
  FEATURE_KEYS: { apiTokens: 'api_tokens', billingHeader: 'billing_header' },
}));

import { SettingsScreen } from '../src/screens/settings/SettingsScreen';

type AlertButton = { text?: string; style?: string; onPress?: () => void };

/** The panes a signed-in athlete gets with both feature flags off. */
const athletePanes = settingsPanesFor('mobile').filter((pane) => pane.flag === null);

describe('SettingsScreen rows', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('draws no glyph in the pane list other than the chevron on each row', () => {
    // Every pane used to carry a 40-square with a Feather icon in it. The row
    // grammar has one glyph, the chevron, and it means "this opens a screen".
    const { getByTestId } = render(<SettingsScreen />);
    const icons = within(getByTestId('settings-pane-list')).getAllByTestId(/^icon-/);
    expect(icons.length).toBe(athletePanes.length);
    for (const icon of icons) {
      expect(icon.props.testID).toBe('icon-chevron-right');
    }
  });

  it('renders each pane\'s hint inside the same row as its name', () => {
    const { getByTestId } = render(<SettingsScreen />);
    for (const pane of athletePanes) {
      const row = within(getByTestId(`settings-pane-${pane.id}`));
      expect(row.getByText(i18n.t(pane.nameKey))).toBeTruthy();
      expect(row.getByText(i18n.t(pane.hintKey))).toBeTruthy();
    }
  });

  it('lays the rows on the ground, with no card behind them', () => {
    const { getByTestId } = render(<SettingsScreen />);
    const list = getByTestId('settings-pane-list');
    expect(list.props.className ?? '').not.toContain('bg-');
    expect(list.props.style?.backgroundColor).toBeUndefined();
    expect(list.props.style?.borderWidth).toBeUndefined();
  });

  it('ends with a quiet sign-out that asks first, and signs out on confirm', () => {
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    const { getByTestId } = render(<SettingsScreen />);

    const signOut = getByTestId('settings-sign-out');
    expect(within(signOut).getByText(i18n.t('app.logOut')).props.className).toContain('text-text-secondary');
    expect(within(signOut).queryByTestId('icon-chevron-right')).toBeNull();

    fireEvent.press(signOut);
    expect(alertSpy).toHaveBeenCalledTimes(1);
    expect(alertSpy.mock.calls[0][0]).toBe(i18n.t('common.logout'));
    expect(alertSpy.mock.calls[0][1]).toBe(i18n.t('app.signOutConfirm'));
    expect(mockLogout).not.toHaveBeenCalled();

    const buttons = alertSpy.mock.calls[0][2] as AlertButton[];
    const confirm = buttons.find((button) => button.style === 'destructive');
    expect(confirm?.text).toBe(i18n.t('common.logout'));
    confirm?.onPress?.();
    expect(mockLogout).toHaveBeenCalledTimes(1);
    alertSpy.mockRestore();
  });
});
