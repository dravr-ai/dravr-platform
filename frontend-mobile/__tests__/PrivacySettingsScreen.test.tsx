// ABOUTME: Unit tests for the Settings privacy screen: the analytics-consent control, and "Who read your data"
// ABOUTME: Consent reads the stored flag and reverts a failed write; Revoke revokes every grant of a client, never a key's or support's

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert, type AlertButton } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { AccessFeed, OAuthGrant, User } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import {
  ACCESS_FEED_DAY_READS_KEYS,
  ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS,
  ACCESS_FEED_KEYS,
  ACCESS_FEED_PEER_DAY_READS_KEYS,
  ACCESS_FEED_PEER_DAY_VIEWS_KEYS,
} from '@pierre/shared-constants';

const mockBack = jest.fn();
let mockSearchParams: Record<string, string> = {};
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: mockBack }),
  useLocalSearchParams: () => mockSearchParams,
}));

const mockUpdateAnalyticsConsent = jest.fn();
const mockListDataAccessRequests = jest.fn();
const mockGetAccessFeed = jest.fn();
const mockListConnectedApps = jest.fn();
const mockRevokeConnectedApp = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    updateAnalyticsConsent: (enabled: boolean) => mockUpdateAnalyticsConsent(enabled),
  },
  accessFeedApi: {
    getAccessFeed: () => mockGetAccessFeed(),
  },
  oauthApi: {
    listConnectedApps: () => mockListConnectedApps(),
    revokeConnectedApp: (id: string) => mockRevokeConnectedApp(id),
  },
  dataAccessRequestsApi: {
    list: () => mockListDataAccessRequests(),
    letter: () => new Promise(() => undefined),
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

let lastQueryClient: QueryClient | null = null;

function renderScreen() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  lastQueryClient = queryClient;
  return render(
    <QueryClientProvider client={queryClient}>
      <PrivacySettingsScreen />
    </QueryClientProvider>,
  );
}

const EMPTY_FEED: AccessFeed = { app_reads: [], coach_days: [], peer_days: [], api_key_days: [], days: 30 };

describe('PrivacySettingsScreen — analytics consent', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSearchParams = {};
    mockListDataAccessRequests.mockResolvedValue([]);
    mockGetAccessFeed.mockResolvedValue(EMPTY_FEED);
    mockListConnectedApps.mockResolvedValue([]);
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

  it('opens the data access request a reminder tap names (carnet#1659)', async () => {
    mockSearchParams = { request: 'r1' };
    mockListDataAccessRequests.mockResolvedValue([
      {
        id: 'r1',
        reference: 'DRV-2026-AB3XZ7',
        controller: 'Acme Fitness',
        controller_contact: 'privacy@acme.example',
        athlete_identifier: 'ana@example.com',
        law: 'gdpr',
        scope: 'access',
        letter_locale: 'en',
        status: 'sent',
        derived_state: null,
        sent_on: '2026-11-10',
        due_on: '2026-12-10',
        extended_until: null,
        answered_on: null,
        created_at: '2026-11-10T15:00:00Z',
        updated_at: '2026-11-10T15:00:00Z',
      },
    ]);
    const { findByTestId } = renderScreen();

    expect(await findByTestId('data-access-detail-status')).toHaveTextContent('Sent');
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
      expect(mockUpdateUser).toHaveBeenCalledWith({ analytics_consent: true, analytics_consent_answered: true });
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
  // card anywhere in the tree (Boreal v2.2, DESIGN.md "Phone").
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

function grant(id: string, clientId: string, scope: string): OAuthGrant {
  return { id, client_id: clientId, client_name: 'Claude', client_host: 'claude.ai', scope, granted_at: '2026-11-01T10:00:00Z', preset: null, max_tier: null, expires_at: null };
}

const FEED: AccessFeed = {
  days: 30,
  app_reads: [
    {
      read_at: '2026-11-20T10:00:00Z',
      kind: 'tool_call',
      tool: 'get_activities',
      tools: ['get_activities'],
      client_id: 'client-claude',
      client_name: 'Claude',
      client_host: 'claude.ai',
      credential: 'oauth_grant',
      transport: 'mcp_http',
      support_read: false,
    },
    {
      read_at: '2026-11-19T09:00:00Z',
      kind: 'tool_call',
      tool: 'get_sleep_sessions',
      tools: ['get_sleep_sessions'],
      client_id: 'client-script',
      client_name: 'Export script',
      client_host: null,
      credential: 'api_key',
      transport: 'mcp_http',
      support_read: false,
    },
    {
      read_at: '2026-11-18T08:00:00Z',
      kind: 'tool_call',
      tool: 'get_athlete',
      tools: ['get_athlete'],
      client_id: 'client-claude',
      client_name: 'Admin console',
      client_host: 'admin.dravr.ai',
      credential: 'oauth_grant',
      transport: 'mcp_http',
      support_read: true,
    },
  ],
  coach_days: [
    {
      day: '2026-11-20',
      coach_name: 'Marie',
      group_id: 'group-1',
      group_name: 'Run Club',
      data_class: 'wellness',
      reads: 2,
      views: 0,
      served_to_assistant: false,
      support_read: false,
    },
  ],
  peer_days: [],
  api_key_days: [],
};

/** The feed with an agent turn, members' days (named, gone, views only) and an API key's days. */
const WIDER_FEED: AccessFeed = {
  ...FEED,
  app_reads: [
    ...FEED.app_reads,
    {
      read_at: '2026-11-17T07:00:00Z',
      kind: 'agent_turn',
      tool: null,
      tools: ['get_activities', 'get_athlete'],
      client_id: 'client-bridge',
      client_name: 'Training bridge',
      client_host: null,
      credential: 'a2a_client_credentials',
      transport: 'a2a',
      support_read: false,
    },
  ],
  peer_days: [
    { day: '2026-11-21', member_name: 'Alice', group_id: 'group-1', group_name: 'Run Club', reads: 2, views: 1, served_to_assistant: true, support_read: false },
    { day: '2026-11-20', member_name: null, group_id: 'group-2', group_name: null, reads: 1, views: 0, served_to_assistant: true, support_read: true },
    { day: '2026-11-19', member_name: 'Bob', group_id: 'group-1', group_name: 'Run Club', reads: 0, views: 3, served_to_assistant: false, support_read: false },
  ],
  api_key_days: [
    { day: '2026-11-21', key_id: 'key-1', key_name: 'Garage sync', key_prefix: 'pk_live_ab12', requests: 4 },
    { day: '2026-11-20', key_id: 'key-1', key_name: 'Garage sync', key_prefix: 'pk_live_ab12', requests: 1 },
  ],
};

describe('PrivacySettingsScreen — who read your data', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSearchParams = {};
    mockListDataAccessRequests.mockResolvedValue([]);
    mockUseAuth.mockReturnValue({ isAuthenticated: true, user: userWithConsent(false) as User, updateUser: mockUpdateUser });
    mockGetAccessFeed.mockResolvedValue(FEED);
    mockListConnectedApps.mockResolvedValue([
      grant('g1', 'client-claude', 'fitness:read'),
      grant('g2', 'client-claude', 'fitness:write'),
    ]);
    mockRevokeConnectedApp.mockResolvedValue(undefined);
  });

  it('lists each app with its calls, and the coach\'s day with its count', async () => {
    const { findAllByTestId, getByTestId, getAllByTestId } = renderScreen();

    const names = await findAllByTestId('access-feed-app-name');
    expect(names.map((name) => name.props.children)).toEqual(['Claude', 'Export script']);
    expect(getByTestId('access-feed-read-get_activities')).toBeTruthy();
    expect(getAllByTestId('access-feed-coach-day')).toHaveLength(1);
    expect(getByTestId('access-feed-coach-name')).toHaveTextContent('Marie');
    expect(getByTestId('access-feed-coach-counts')).toHaveTextContent(
      i18n.t(ACCESS_FEED_DAY_READS_KEYS.many, { count: 2, group: 'Run Club' }),
    );
  });

  it('revokes every grant of that client after the confirm, and rereads the shared keys', async () => {
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    const { findAllByTestId } = renderScreen();
    const invalidate = jest.spyOn(lastQueryClient as QueryClient, 'invalidateQueries');

    const [revoke] = await findAllByTestId('access-feed-revoke');
    fireEvent.press(revoke);
    const buttons = alertSpy.mock.calls[0][2] as AlertButton[];
    buttons.find((button) => button.style === 'destructive')?.onPress?.();

    await waitFor(() => expect(mockRevokeConnectedApp).toHaveBeenCalledTimes(2));
    expect(mockRevokeConnectedApp.mock.calls.map(([id]) => id)).toEqual(['g1', 'g2']);
    await waitFor(() =>
      expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual(
        expect.arrayContaining([['oauth', 'connected-apps'], ['access-feed']]),
      ),
    );
    alertSpy.mockRestore();
  });

  it('offers one Revoke only: none to an API key, which it points to its settings, none to support, which it does not name', async () => {
    const { findAllByTestId, getByTestId, queryByText } = renderScreen();

    expect(await findAllByTestId('access-feed-revoke')).toHaveLength(1);
    expect(getByTestId('access-feed-key-hint')).toBeTruthy();
    expect(getByTestId('access-feed-support')).toBeTruthy();
    expect(queryByText('Admin console')).toBeNull();
  });

  it('reads an agent turn as one row that names no tool', async () => {
    mockGetAccessFeed.mockResolvedValue(WIDER_FEED);
    const { findByTestId, queryByText } = renderScreen();

    const turn = await findByTestId('access-feed-read-agent-turn');
    expect(turn).toHaveTextContent(i18n.t(ACCESS_FEED_KEYS.agentTurnRow), { exact: false });
    expect(queryByText(/null/)).toBeNull();
  });

  it('lists each member\'s day, a gone member and group by their stand-ins, and a views-only day by name', async () => {
    mockGetAccessFeed.mockResolvedValue(WIDER_FEED);
    const { findAllByTestId, getAllByTestId } = renderScreen();

    const lines = await findAllByTestId('access-feed-peer-line');
    expect(lines.map((line) => line.props.children)).toEqual([
      i18n.t(ACCESS_FEED_PEER_DAY_READS_KEYS.many, { member: 'Alice', count: 2, group: 'Run Club' }),
      i18n.t(ACCESS_FEED_PEER_DAY_READS_KEYS.one, {
        member: i18n.t(ACCESS_FEED_KEYS.formerMember),
        count: 1,
        group: i18n.t('humanCoach.readLog.unknownGroup'),
      }),
      'Bob',
    ]);
    expect(getAllByTestId('access-feed-peer-views').map((views) => views.props.children)).toEqual([
      i18n.t(ACCESS_FEED_PEER_DAY_VIEWS_KEYS.one, { count: 1, group: 'Run Club' }),
      i18n.t(ACCESS_FEED_PEER_DAY_VIEWS_KEYS.many, { count: 3, group: 'Run Club' }),
    ]);
  });

  it('marks a member\'s day served to their AI tool, and support\'s day as support\'s alone', async () => {
    mockGetAccessFeed.mockResolvedValue(WIDER_FEED);
    const { findAllByTestId, getAllByTestId } = renderScreen();

    const days = await findAllByTestId('access-feed-peer-day');
    expect(days).toHaveLength(3);
    // Alice's day went to her AI tool; the gone member's was support's, which wins over the AI-tool mark.
    expect(getAllByTestId('access-feed-peer-served').map((mark) => mark.props.children)).toEqual([
      i18n.t(ACCESS_FEED_KEYS.peerServedToAssistant),
    ]);
    expect(getAllByTestId('access-feed-peer-support').map((mark) => mark.props.children)).toEqual([
      i18n.t(ACCESS_FEED_KEYS.peerSupportRead),
    ]);
  });

  it('lists an API key\'s days under its name and prefix, pointing to its settings with no Revoke', async () => {
    mockGetAccessFeed.mockResolvedValue({ ...EMPTY_FEED, api_key_days: WIDER_FEED.api_key_days });
    const { findByTestId, getByTestId, getAllByTestId, queryByTestId, getByText } = renderScreen();

    expect(await findByTestId('access-feed-api-key-name')).toHaveTextContent('Garage sync');
    expect(getByText(`pk_live_ab12 · ${i18n.t('accessFeed.credential.apiKey')}`)).toBeTruthy();
    expect(getByTestId('access-feed-key-hint')).toBeTruthy();
    expect(getAllByTestId('access-feed-api-key-requests').map((count) => count.props.children)).toEqual([
      i18n.t(ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS.many, { count: 4 }),
      i18n.t(ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS.one, { count: 1 }),
    ]);
    expect(queryByTestId('access-feed-revoke')).toBeNull();
    expect(queryByTestId('access-feed-empty')).toBeNull();
  });

  it('lists a feed that holds only a member\'s reads instead of calling it empty', async () => {
    mockGetAccessFeed.mockResolvedValue({ ...EMPTY_FEED, peer_days: WIDER_FEED.peer_days.slice(0, 1) });
    const { findByTestId, queryByTestId } = renderScreen();

    expect(await findByTestId('access-feed-peer-day')).toBeTruthy();
    expect(queryByTestId('access-feed-empty')).toBeNull();
  });

  it('says nothing read the data when every list is empty', async () => {
    mockGetAccessFeed.mockResolvedValue(EMPTY_FEED);
    const { findByTestId, queryByTestId } = renderScreen();

    expect(await findByTestId('access-feed-empty')).toBeTruthy();
    expect(queryByTestId('access-feed-app-name')).toBeNull();
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
