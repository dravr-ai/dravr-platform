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
  client_id: 'Claude Desktop',
  scope: 'fitness:read',
  granted_at: '2026-08-01T10:00:00Z',
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
    await i18n.changeLanguage('en');
  });

  beforeEach(() => {
    jest.clearAllMocks();
    mockListConnectedApps.mockResolvedValue([GRANT]);
    mockRevokeConnectedApp.mockResolvedValue(undefined);
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
