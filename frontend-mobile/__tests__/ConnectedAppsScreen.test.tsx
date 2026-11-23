// ABOUTME: Pins the connected-apps screen to the catalogue the web card already reads, in French too
// ABOUTME: Its explanatory paragraph was a hardcoded English copy sitting beside that key

import React from 'react';
import { Alert } from 'react-native';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
}));

const mockListConnectedApps = jest.fn();
const mockRevokeConnectedApp = jest.fn();
jest.mock('../src/services/api', () => ({
  oauthApi: {
    listConnectedApps: () => mockListConnectedApps(),
    revokeConnectedApp: (id: string) => mockRevokeConnectedApp(id),
  },
}));

import { ConnectedAppsScreen } from '../src/screens/settings/ConnectedAppsScreen';

const GRANT = {
  id: 'grant-1',
  client_id: 'mcp_client_0123456789abcdef',
  client_name: 'Claude Desktop' as string | null,
  client_host: null as string | null,
  scope: 'fitness:read',
  granted_at: '2026-08-01T10:00:00Z',
  preset: null as string | null,
  max_tier: null as 'T0' | 'T1' | 'T2' | 'T3' | null,
  expires_at: null as string | null,
};

function renderScreen(): ReturnType<typeof render> {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ConnectedAppsScreen />
    </QueryClientProvider>,
  );
}

describe('ConnectedAppsScreen', () => {
  // The suite pins English (jest.setup.js); the French case asks for the
  // product's own default locale and hands it back afterwards.
  afterEach(async () => {
    jest.restoreAllMocks();
    await i18n.changeLanguage('en');
  });

  beforeEach(() => {
    jest.clearAllMocks();
    mockListConnectedApps.mockResolvedValue([GRANT]);
    mockRevokeConnectedApp.mockResolvedValue(undefined);
  });

  it('names the app with its host, never its opaque client id', async () => {
    mockListConnectedApps.mockResolvedValue([{ ...GRANT, client_host: 'claude.ai' }]);
    renderScreen();
    await waitFor(() => expect(screen.getByText('Claude Desktop')).toBeTruthy());
    expect(screen.getByText(/claude\.ai · fitness:read/)).toBeTruthy();
    expect(screen.queryByText(GRANT.client_id)).toBeNull();
  });

  it('falls back to the client id for an app with no registered name', async () => {
    mockListConnectedApps.mockResolvedValue([{ ...GRANT, client_name: null, client_host: null }]);
    renderScreen();
    await waitFor(() => expect(screen.getByText(GRANT.client_id)).toBeTruthy());
  });

  it('explains the screen from the shared catalogue key, not a paragraph of its own', async () => {
    renderScreen();
    await waitFor(() => expect(screen.getByText(i18n.t('tokens.connectedAppsHint'))).toBeTruthy());
  });

  it('renders that paragraph in French, which is the language the rest of the screen speaks', async () => {
    await i18n.changeLanguage('fr');
    const french = i18n.t('tokens.connectedAppsHint');
    // Asserted on the real French copy, not on "not English": an empty render
    // would satisfy a negative assertion just as well.
    expect(french).toMatch(/OAuth/);
    expect(french).not.toMatch(/Apps you approved/);

    renderScreen();
    await waitFor(() => expect(screen.getByText(french)).toBeTruthy());
  });

  it('dates the grant in one French sentence, in the app language rather than the device locale', async () => {
    await i18n.changeLanguage('fr');
    renderScreen();
    await waitFor(() =>
      expect(screen.getByText('fitness:read · Connecté le 1 août 2026')).toBeTruthy(),
    );
  });

  it('says when a grant expires, and says nothing of expiry for one that never does', async () => {
    // Before the grant's end, so it reads as live.
    jest.spyOn(Date, 'now').mockReturnValue(Date.parse('2026-08-15T00:00:00Z'));
    await i18n.changeLanguage('fr');
    mockListConnectedApps.mockResolvedValue([
      { ...GRANT, preset: 'sleep_coach', max_tier: 'T1', expires_at: '2026-09-01T10:00:00Z' },
      { ...GRANT, id: 'grant-2', client_name: 'Plain client' },
    ]);
    renderScreen();
    const sleep = i18n.t('tokens.connectedAppsPresetSleepCoach');
    await waitFor(() =>
      expect(
        screen.getByText(`${sleep} · Connecté le 1 août 2026 · Expire le 1 sept. 2026`),
      ).toBeTruthy(),
    );
    expect(screen.getByText('fitness:read · Connecté le 1 août 2026')).toBeTruthy();
  });

  it('shows a grant whose end has passed as expired in the muted hint, and keeps Revoke', async () => {
    jest.spyOn(Date, 'now').mockReturnValue(Date.parse('2026-11-22T00:00:00Z'));
    mockListConnectedApps.mockResolvedValue([
      { ...GRANT, preset: 'sleep_coach', max_tier: 'T1', expires_at: '2026-09-01T10:00:00Z' },
    ]);
    renderScreen();
    const expired = i18n.t('providers.expired');
    expect(expired).not.toEqual('providers.expired');
    const hint = await screen.findByTestId('connected-app-expired-grant-1');
    expect(hint.props.children).toBe(expired);
    expect(hint.props.className).toContain('text-text-tertiary');
    const sleep = i18n.t('tokens.connectedAppsPresetSleepCoach');
    // The past date no longer reads like a live grant's end.
    expect(screen.getByText(`${sleep} · Connected Aug 1, 2026`)).toBeTruthy();
    expect(screen.queryByText(/Expires/)).toBeNull();
    expect(screen.getByTestId('revoke-grant-1')).toBeTruthy();
  });

  it('names a preset grant by its preset, and a plain grant by its scope', async () => {
    mockListConnectedApps.mockResolvedValue([
      { ...GRANT, preset: 'training_coach', max_tier: 'T1' },
      { ...GRANT, id: 'grant-2', client_name: 'Sleep app', preset: 'sleep_coach', max_tier: 'T1' },
      { ...GRANT, id: 'grant-3', client_name: 'Later app', preset: 'clinician_summary', max_tier: 'T2' },
    ]);
    renderScreen();
    const training = i18n.t('tokens.connectedAppsPresetTrainingCoach');
    const sleep = i18n.t('tokens.connectedAppsPresetSleepCoach');
    expect(training).not.toEqual('tokens.connectedAppsPresetTrainingCoach');
    expect(sleep).not.toEqual('tokens.connectedAppsPresetSleepCoach');
    await waitFor(() => expect(screen.getByText(new RegExp(`^${training} · `))).toBeTruthy());
    expect(screen.getByText(new RegExp(`^${sleep} · `))).toBeTruthy();
    // A preset this build does not know yet falls back to the scope, never a raw key.
    expect(screen.getByText(/^fitness:read · /)).toBeTruthy();
  });

  it('names the app it is about to revoke from the catalogue, in the label and the confirm', async () => {
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    const label = i18n.t('app.revokeAppLabel', { app: 'Claude Desktop' });
    renderScreen();

    await waitFor(() => expect(screen.getByLabelText(label)).toBeTruthy());
    fireEvent.press(screen.getByLabelText(label));

    expect(alertSpy).toHaveBeenCalledWith(
      i18n.t('app.revokeAccessQ'),
      i18n.t('app.confirmRevokeAppAccess', { app: 'Claude Desktop' }),
      expect.any(Array),
    );
    alertSpy.mockRestore();
  });

  it('carries the hint in every locale the switcher offers', async () => {
    // i18next returns the key itself on a miss, so a locale that never got the
    // sentence renders `tokens.connectedAppsHint` to the athlete.
    for (const locale of ['fr', 'en', 'es', 'de', 'pt']) {
      await i18n.changeLanguage(locale);
      expect(i18n.t('tokens.connectedAppsHint')).not.toEqual('tokens.connectedAppsHint');
      expect(i18n.t('app.revokeAppLabel')).not.toEqual('app.revokeAppLabel');
    }
  });
});
