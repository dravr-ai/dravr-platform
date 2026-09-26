// ABOUTME: Unit tests for the analytics-consent control on the Settings privacy screen
// ABOUTME: Pins that it reads the stored flag, writes through userApi, and reverts when the write fails

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { User } from '@pierre/shared-types';

const mockBack = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: mockBack }),
}));

const mockUpdateAnalyticsConsent = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    updateAnalyticsConsent: (enabled: boolean) => mockUpdateAnalyticsConsent(enabled),
  },
}));

const mockUpdateUser = jest.fn();
const mockUseAuth = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockUseAuth(),
}));

import { PrivacySettingsScreen } from '../src/screens/settings/PrivacySettingsScreen';
import { networkFailure } from '../integration/app/helpers/apiRefusal';

const userWithConsent = (consent: boolean): Partial<User> => ({
  id: 'user-1',
  email: 'mobiletest@pierre.dev',
  is_admin: false,
  role: 'user',
  user_status: 'active',
  analytics_consent: consent,
});

function renderScreen() {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <PrivacySettingsScreen />
    </QueryClientProvider>,
  );
}

describe('PrivacySettingsScreen — analytics consent', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseAuth.mockReturnValue({
      isAuthenticated: true,
      user: userWithConsent(false) as User,
      updateUser: mockUpdateUser,
    });
    mockUpdateAnalyticsConsent.mockResolvedValue({ message: 'Updated', enabled: true });
    mockUpdateUser.mockResolvedValue(undefined);
  });

  it('renders under Settings with no header of its own', () => {
    // The native header names the pane and carries the way back (Boreal v2.2
    // D2); a title or a back button drawn here is a second header idiom.
    const { getByTestId, queryByTestId, queryByText } = renderScreen();
    expect(getByTestId('privacy-settings-screen')).toBeTruthy();
    expect(queryByText('Privacy & Data')).toBeNull();
    expect(queryByTestId('back-button')).toBeNull();
  });

  it('seeds the switch from the stored consent flag', () => {
    mockUseAuth.mockReturnValue({
      isAuthenticated: true,
      user: userWithConsent(true) as User,
      updateUser: mockUpdateUser,
    });
    const { getByTestId } = renderScreen();
    expect(getByTestId('analytics-consent-switch').props.value).toBe(true);
  });

  it('persists an opt-in and reflects it on the user record', async () => {
    const { getByTestId } = renderScreen();
    expect(getByTestId('analytics-consent-switch').props.value).toBe(false);

    fireEvent(getByTestId('analytics-consent-switch'), 'valueChange', true);

    await waitFor(() => {
      expect(mockUpdateAnalyticsConsent).toHaveBeenCalledWith(true);
    });
    await waitFor(() => {
      expect(mockUpdateUser).toHaveBeenCalledWith({ analytics_consent: true });
    });
    expect(getByTestId('analytics-consent-switch').props.value).toBe(true);
  });

  it('reverts the switch when the write fails', async () => {
    // The important one. An optimistic switch that stays flipped after a failed
    // write tells the user their data sharing is off while it is still on.
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockUpdateAnalyticsConsent.mockRejectedValueOnce(networkFailure());

    const { getByTestId } = renderScreen();
    fireEvent(getByTestId('analytics-consent-switch'), 'valueChange', true);

    await waitFor(() => {
      expect(alertSpy).toHaveBeenCalledWith('Could not save preference', 'Network error. Check your connection.');
    });
    await waitFor(() => {
      expect(getByTestId('analytics-consent-switch').props.value).toBe(false);
    });
    expect(mockUpdateUser).not.toHaveBeenCalled();
    alertSpy.mockRestore();
  });

  // Turns red if the cards come back: three sections in a gap-8 column, the
  // switch as the first section's action, and no icon circle, pill or filled
  // card anywhere in the tree (Boreal v2.2, DESIGN.md §10).
  it('lays the consent switch and the two promise lists out as sections, with no card or icon circle', () => {
    const { getByTestId, toJSON } = renderScreen();

    for (const id of ['privacy-section-analytics', 'privacy-section-collected', 'privacy-section-never']) {
      const section = getByTestId(id);
      expect(section.props.className).not.toContain('bg-');
      expect(section.props.className).not.toContain('border');
      expect(section.props.style).toBeUndefined();
    }
    expect(getByTestId('analytics-consent-switch')).toBeTruthy();

    const serialised = JSON.stringify(toJSON());
    expect(serialised).not.toContain('rounded-full');
    expect(serialised).toContain('"className":"gap-8"');
    expect(serialised).not.toContain('mt-6 mb-2 ml-2');
    const styles = allStyles(rootOf(toJSON));
    expect(styles.some((s) => typeof s.borderRadius === 'number' && s.borderRadius >= 999)).toBe(false);
    // The card recipe: a fill with a hairline around it.
    expect(styles.some((s) => s.backgroundColor !== undefined && s.borderWidth !== undefined)).toBe(false);
  });
});

type Json = { type: string; props: Record<string, unknown>; children: Array<Json | string> | null };

/** Every style object in a rendered tree, flat or nested arrays alike. */
function allStyles(node: Json | string | null | undefined, out: Array<Record<string, unknown>> = []) {
  if (!node || typeof node === 'string') return out;
  const flatten = (style: unknown): void => {
    if (Array.isArray(style)) style.forEach(flatten);
    else if (style && typeof style === 'object') out.push(style as Record<string, unknown>);
  };
  flatten(node.props.style);
  for (const child of node.children ?? []) allStyles(child, out);
  return out;
}

/** The rendered tree as one node, whichever shape `toJSON()` returned. */
function rootOf(tree: ReturnType<typeof render>['toJSON']): Json {
  const rendered = tree() as Json | Json[] | null;
  if (!rendered) throw new Error('nothing rendered');
  return Array.isArray(rendered) ? rendered[0] : rendered;
}
