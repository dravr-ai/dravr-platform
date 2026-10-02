// ABOUTME: Asserts the phone's settings list is derived from the shared pane declaration and serves every pane
// ABOUTME: Mobile shipped one long scroll while web shipped ten panes, and nothing compared the two

import fs from 'fs';
import path from 'path';
import React from 'react';
import { fireEvent, render, within } from '@testing-library/react-native';
import type { User } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import { SETTINGS_PANES, settingsPanesFor } from '@pierre/shared-constants';

const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, back: jest.fn() }),
  useFocusEffect: () => undefined,
}));

jest.mock('../src/services/api', () => ({
  userApi: { getMcpTokens: jest.fn().mockResolvedValue({ tokens: [] }) },
  oauthApi: { getProvidersStatus: jest.fn().mockResolvedValue({ providers: [] }) },
}));

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
    logout: jest.fn(),
    isAuthenticated: true,
    updateUser: jest.fn(),
  }),
}));

// Every gate open, so the list under test is the whole declaration: a pane
// behind a flag is still a pane the phone has to be able to serve.
jest.mock('../src/hooks/useFeatureFlags', () => ({
  useFeatureFlags: () => ({ flags: { api_tokens: true, billing_header: true }, known: [], isLoading: false, isError: false }),
  FEATURE_KEYS: { apiTokens: 'api_tokens', billingHeader: 'billing_header' },
}));
jest.mock('../src/constants/features', () => ({
  ...jest.requireActual('../src/constants/features'),
  BILLING_ENABLED: true,
}));

import { SettingsScreen } from '../src/screens/settings/SettingsScreen';

/**
 * Not a diff between web's tabs and mobile's rows. It checks one client
 * against the single declaration: adding a pane means editing
 * `SETTINGS_PANES` first, and this says whether mobile has caught up.
 *
 * Why the routes are checked on disk: expo-router's route table IS the file
 * tree under app/ — a route exists exactly when its file does, and there is
 * no other artefact to ask. The settings list itself is rendered.
 */
const APP_DIR = path.join(__dirname, '..', 'app');

/** Turn an expo-router path into the file that should serve it. */
function routeFileCandidates(route: string): string[] {
  const rel = route.replace(/^\//, '');
  return [
    path.join(APP_DIR, `${rel}.tsx`),
    path.join(APP_DIR, rel, 'index.tsx'),
    path.join(APP_DIR, rel, '_layout.tsx'),
  ];
}

describe('settings pane parity — mobile', () => {
  const mobilePanes = settingsPanesFor('mobile');

  it('serves every pane web serves', () => {
    // The audit's finding, as an assertion: web had ten named panes and the
    // phone had one scroll, so "privacy is missing on mobile" was a reasonable
    // reading of a screen where privacy sat 1,200pt down.
    const webOnly = SETTINGS_PANES.filter((pane) => pane.web !== null && pane.mobile === null);
    expect(webOnly.map((pane) => pane.id)).toEqual([]);
  });

  it('records a reason whenever a platform deliberately lacks a pane', () => {
    // A null without a reason is indistinguishable from an oversight.
    const unexplained = SETTINGS_PANES.filter(
      (pane) => (pane.web === null || pane.mobile === null) && !pane.why,
    );
    expect(unexplained.map((pane) => pane.id)).toEqual([]);
  });

  it.each(mobilePanes.map((pane) => [pane.id, pane.mobile as string]))(
    'implements %s at %s',
    (_id, route) => {
      const found = routeFileCandidates(route).some((candidate) => fs.existsSync(candidate));
      // jest's `expect` takes a single argument, so the explanation rides in
      // the compared value where the failure diff will actually print it.
      const outcome = found
        ? 'implemented'
        : `MISSING — no expo-router file serves ${route}. Build the pane, or set ` +
          'mobile: null in SETTINGS_PANES with a "why" if mobile should not have it.';
      expect(outcome).toBe('implemented');
    },
  );

  it.each(
    SETTINGS_PANES.flatMap((pane) =>
      Object.entries(pane.mobileScreens ?? {}).map(([section, route]) => [pane.id, section, route]),
    ),
  )('implements the %s pane\'s %s section screen at %s', (_pane, _section, route) => {
    const found = routeFileCandidates(route as string).some((candidate) => fs.existsSync(candidate));
    const outcome = found
      ? 'implemented'
      : `MISSING — no expo-router file serves ${route}. Build the screen, or drop ` +
        'the section from the pane\'s mobileScreens.';
    expect(outcome).toBe('implemented');
  });

  it('builds its rows from the declaration rather than a second hand-written list', () => {
    // A hand-written list is how the grouping drifted the first time: usage
    // stood alone here and sat inside Account on web, and both were correct
    // according to their own source. So the rendered list is compared with
    // the declaration whole — same panes, same order, the declaration's own
    // name and hint on each row, and each row leading where it says. A list
    // kept by hand passes this only for as long as someone keeps it equal by
    // hand, and fails on the first pane the declaration gains.
    //
    // The rows are read off the rendered screen: a search of its source for
    // `settingsPanesFor('mobile')` passes a second list beside that call.
    const view = render(<SettingsScreen />);
    const rows = within(view.getByTestId('settings-pane-list'))
      .getAllByTestId(/^settings-pane-/)
      .map((row) => row.props.testID as string)
      // The list's own id and each row's inner pressable share the prefix.
      .filter((id) => id !== 'settings-pane-list' && !id.endsWith('-inner'));
    expect(rows).toEqual(mobilePanes.map((pane) => `settings-pane-${pane.id}`));
    expect(mobilePanes.length).toBeGreaterThanOrEqual(11);

    for (const pane of mobilePanes) {
      const row = within(view.getByTestId(`settings-pane-${pane.id}`));
      expect(row.getByText(i18n.t(pane.nameKey))).toBeTruthy();
      expect(row.getByText(i18n.t(pane.hintKey))).toBeTruthy();
      mockPush.mockClear();
      fireEvent.press(view.getByTestId(`settings-pane-${pane.id}`));
      expect(mockPush.mock.calls).toEqual([[pane.mobile]]);
    }
  });

  it('no longer declares the retired AI-provider pane', () => {
    expect(SETTINGS_PANES.map((pane) => String(pane.id))).not.toContain('ai-provider');
    expect(SETTINGS_PANES.some((pane) => pane.mobile?.endsWith('/ai-provider'))).toBe(false);
    expect(fs.existsSync(path.join(APP_DIR, '(app)/(tabs)/(settings)/ai-provider.tsx'))).toBe(false);
  });
});
