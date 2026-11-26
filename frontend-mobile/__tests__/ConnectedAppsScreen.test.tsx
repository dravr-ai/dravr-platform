// ABOUTME: Pins the connected-apps screen to the catalogue the web card already reads, in French too, and to the shared revoke
// ABOUTME: Its explanatory paragraph was a hardcoded English copy; its revoke now rereads the key the access feed shares (carnet#1543)

import React from 'react';
import { Alert, type AlertButton } from 'react-native';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
}));

const mockListConnectedApps = jest.fn();
const mockRevokeConnectedApp = jest.fn();
const mockListStandingPolicies = jest.fn();
const mockRemoveStandingPolicy = jest.fn();
const mockListGrantReceipts = jest.fn();
jest.mock('../src/services/api', () => ({
  oauthApi: {
    listConnectedApps: () => mockListConnectedApps(),
    revokeConnectedApp: (id: string) => mockRevokeConnectedApp(id),
    listStandingPolicies: () => mockListStandingPolicies(),
    removeStandingPolicy: (id: string) => mockRemoveStandingPolicy(id),
    listGrantReceipts: (page?: { limit?: number; offset?: number }) => mockListGrantReceipts(page),
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

let lastQueryClient: QueryClient | null = null;

function renderScreen(): ReturnType<typeof render> {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  lastQueryClient = queryClient;
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
    mockListStandingPolicies.mockResolvedValue([]);
    mockRemoveStandingPolicy.mockResolvedValue(undefined);
    mockListGrantReceipts.mockResolvedValue([]);
  });

  // carnet#1544: the apps the athlete always allows, each with Stop, and a
  // mark on their rows. Revoking such an app's grant also ends its policy.
  const POLICY = {
    id: 'policy-1',
    client_id: GRANT.client_id,
    client_name: 'Claude Desktop' as string | null,
    client_host: null as string | null,
    preset: 'training_coach',
    created_at: '2026-08-02T10:00:00Z',
  };

  it('marks an always-allowed app, revokes it with the plain confirmation, and its policy leaves the list', async () => {
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockListStandingPolicies.mockResolvedValue([POLICY]);
    renderScreen();
    const always = i18n.t('tokens.connectedAppsAlwaysAllowed');
    expect(always).not.toEqual('tokens.connectedAppsAlwaysAllowed');
    await waitFor(() =>
      expect(screen.getByText(`fitness:read · ${always} · Connected Aug 1, 2026`)).toBeTruthy(),
    );
    fireEvent.press(screen.getByTestId(`revoke-${GRANT.id}`));
    expect(alertSpy).toHaveBeenCalledWith(
      i18n.t('app.revokeAccessQ'),
      i18n.t('app.confirmRevokeAppAccess', { app: 'Claude Desktop' }),
      expect.any(Array),
    );

    // The server ends the grant and the app's policy with it; the shared
    // revoke rereads both.
    mockListConnectedApps.mockResolvedValue([]);
    mockListStandingPolicies.mockResolvedValue([]);
    const buttons = alertSpy.mock.calls[0][2] as AlertButton[];
    buttons.find((button) => button.style === 'destructive')?.onPress?.();

    await waitFor(() => expect(mockRevokeConnectedApp).toHaveBeenCalledWith(GRANT.id));
    await waitFor(() => expect(screen.getByTestId('standing-policies-empty')).toBeTruthy());
    expect(mockRemoveStandingPolicy).not.toHaveBeenCalled();
    alertSpy.mockRestore();
  });

  it('lists the always-allowed apps and stops one after asking, rereading the policies', async () => {
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockListStandingPolicies.mockResolvedValue([POLICY]);
    renderScreen();
    const training = i18n.t('tokens.connectedAppsPresetTrainingCoach');
    const since = i18n.t('tokens.standingPolicySince', { date: 'Aug 2, 2026' });
    await waitFor(() => expect(screen.getByText(`${training} · ${since}`)).toBeTruthy());

    fireEvent.press(screen.getByLabelText(i18n.t('tokens.standingPolicyStopLabel', { app: 'Claude Desktop' })));
    expect(alertSpy).toHaveBeenCalledWith(
      i18n.t('tokens.standingPolicyStopQ'),
      i18n.t('tokens.standingPolicyStopConfirm', { app: 'Claude Desktop' }),
      expect.any(Array),
    );
    // The reread after the removal finds the policy gone.
    mockListStandingPolicies.mockResolvedValue([]);
    const buttons = alertSpy.mock.calls[0][2] as AlertButton[];
    buttons.find((button) => button.style === 'destructive')?.onPress?.();

    await waitFor(() => expect(mockRemoveStandingPolicy).toHaveBeenCalledWith(POLICY.id));
    await waitFor(() => expect(screen.getByTestId('standing-policies-empty')).toBeTruthy());
    expect(mockRevokeConnectedApp).not.toHaveBeenCalled();
    alertSpy.mockRestore();
  });

  it('lists the consent history by app and outcome, a deleted app by its id, and pages older receipts', async () => {
    const receipt = (id: string, event: string, actor: string, clientId: string, approval = {}) => ({
      id,
      client_grant_id: `grant-${id}`,
      event,
      actor,
      at: '2026-11-20T10:00:00Z',
      payload: { grant: { client_id: clientId }, approval: { actor, ...approval } },
      payload_sha256: 'ab',
      jws: null,
      logged_at: null,
    });
    const firstPage = [
      receipt('r1', 'issue', 'auto', GRANT.client_id, { standing_policy_ref: { id: 'p1' } }),
      receipt('r2', 'revoke', 'user', 'client-gone'),
      ...Array.from({ length: 48 }, (_, n) => receipt(`f${n}`, 'issue', 'user', GRANT.client_id)),
    ];
    mockListGrantReceipts.mockImplementation(async (page?: { offset?: number }) =>
      (page?.offset ?? 0) === 0 ? firstPage : [receipt('r99', 'expire', 'auto', GRANT.client_id)],
    );
    renderScreen();

    const renewed = await screen.findByTestId('grant-receipt-r1');
    expect(renewed).toHaveTextContent(/Claude Desktop/);
    expect(renewed).toHaveTextContent(new RegExp(i18n.t('tokens.grantEventRenewed')));
    const ended = screen.getByTestId('grant-receipt-r2');
    expect(ended).toHaveTextContent(/client-gone/);
    expect(ended).toHaveTextContent(new RegExp(i18n.t('tokens.grantEventEndedByYou')));
    expect(mockListGrantReceipts).toHaveBeenCalledWith({ limit: 50, offset: 0 });

    fireEvent.press(screen.getByTestId('grant-history-older'));
    const expired = await screen.findByTestId('grant-receipt-r99');
    expect(expired).toHaveTextContent(new RegExp(i18n.t('tokens.grantEventExpired')));
    expect(mockListGrantReceipts).toHaveBeenCalledWith({ limit: 50, offset: 50 });
    expect(screen.queryByTestId('grant-history-older')).toBeNull();
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

  it('revokes through the shared revoke, which rereads the key the access feed shares, and the app leaves', async () => {
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockListConnectedApps.mockResolvedValueOnce([GRANT]).mockResolvedValue([]);
    renderScreen();
    const invalidate = jest.spyOn(lastQueryClient as QueryClient, 'invalidateQueries');

    fireEvent.press(await screen.findByTestId(`revoke-${GRANT.id}`));
    const buttons = alertSpy.mock.calls[0][2] as AlertButton[];
    buttons.find((button) => button.style === 'destructive')?.onPress?.();

    await waitFor(() => expect(mockRevokeConnectedApp).toHaveBeenCalledWith(GRANT.id));
    await waitFor(() => expect(screen.getByTestId('connected-apps-empty')).toBeTruthy());
    expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual(
      expect.arrayContaining([['oauth', 'connected-apps'], ['access-feed']]),
    );
    alertSpy.mockRestore();
  });

  it('lists a one-off question under its app and revokes it alone, the app standing', async () => {
    const stepped = { ...GRANT, step_up: { id: 'child-1', expires_at: '2026-11-25T11:00:00Z' } };
    mockListConnectedApps.mockResolvedValueOnce([stepped]).mockResolvedValue([{ ...stepped, step_up: null }]);
    renderScreen();

    expect(await screen.findByTestId(`connected-app-step-up-${GRANT.id}`)).toBeTruthy();
    const worded = i18n.t('tokens.connectedAppsStepUpChild', { time: 'TIME' });
    expect(worded).not.toEqual('tokens.connectedAppsStepUpChild');
    expect(screen.getByText(new RegExp(worded.split('TIME')[0].trim()))).toBeTruthy();
    // A screen reader hears which app the second Revoke belongs to and what it ends.
    const childRevoke = screen.getByTestId(`revoke-step-up-${GRANT.id}`);
    expect(childRevoke.props.accessibilityLabel).toContain('Claude Desktop');
    expect(childRevoke.props.accessibilityLabel).toContain(worded.split('TIME')[0].trim());
    expect(childRevoke.props.accessibilityLabel).not.toBe(
      screen.getByTestId(`revoke-${GRANT.id}`).props.accessibilityLabel,
    );
    fireEvent.press(childRevoke);

    await waitFor(() => expect(mockRevokeConnectedApp).toHaveBeenCalledWith('child-1'));
    expect(mockRevokeConnectedApp).not.toHaveBeenCalledWith(GRANT.id);
    await waitFor(() => expect(screen.queryByTestId(`connected-app-step-up-${GRANT.id}`)).toBeNull());
    expect(screen.getByText('Claude Desktop')).toBeTruthy();
  });

  it('says what a one-off question reads and on what the app\'s word stands', async () => {
    mockListConnectedApps.mockResolvedValue([
      {
        ...GRANT,
        step_up: {
          id: 'child-2',
          expires_at: '2026-11-25T11:00:00Z',
          streams: ['labs.summary'],
          host_profile_basis: 'declared_cimd',
        },
      },
    ]);
    renderScreen();

    expect(await screen.findByTestId(`connected-app-step-up-${GRANT.id}`)).toBeTruthy();
    const subtitle = `${i18n.t('tokens.connectedAppsStepUpStreams', {
      streams: i18n.t('oauth.stepUp.stream.labs'),
    })} · ${i18n.t('tokens.connectedAppsStepUpDeclaredCimd')}`;
    expect(screen.getByText(subtitle)).toBeTruthy();
  });

  it('says a refused revoke failed, from the catalogue', async () => {
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockRevokeConnectedApp.mockRejectedValue(new Error('refused'));
    renderScreen();

    fireEvent.press(await screen.findByTestId(`revoke-${GRANT.id}`));
    const buttons = alertSpy.mock.calls[0][2] as AlertButton[];
    buttons.find((button) => button.style === 'destructive')?.onPress?.();

    await waitFor(() =>
      expect(alertSpy).toHaveBeenCalledWith(i18n.t('app.couldNotRevokeApp'), expect.any(String)),
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
