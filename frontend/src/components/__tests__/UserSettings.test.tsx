// ABOUTME: Unit tests for the UserSettings component
// ABOUTME: Tests tab navigation, profile display, change password modal, and about tab
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import UserSettings from '../UserSettings';

// Mock lazy-loaded components
vi.mock('../A2AClientList', () => ({
  default: () => <div data-testid="a2a-client-list">A2A Clients</div>,
}));

vi.mock('../CreateA2AClient', () => ({
  default: () => (
    <div data-testid="create-a2a-client">Create A2A Client</div>
  ),
}));

vi.mock('../LlmSettingsTab', () => ({
  default: () => <div data-testid="llm-settings">LLM Settings</div>,
}));

vi.mock('../MessagingSettingsTab', () => ({
  default: () => <div data-testid="messaging-settings">Messaging Settings</div>,
}));

vi.mock('../../hooks/useUsageStatus', () => ({
  useUsageStatus: () => ({
    data: {
      daily: {
        messages: { allowed: true, current: 42, limit: 100, warning: false, burst_zone: false, resets_at: '2026-02-19T05:00:00Z' },
        tokens: { allowed: true, current: 145000, limit: 500000, warning: false, burst_zone: false, resets_at: '2026-02-19T05:00:00Z' },
        tool_calls: { allowed: true, current: 10, limit: 50, warning: false, burst_zone: false, resets_at: '2026-02-19T05:00:00Z' },
      },
      weekly: {
        messages: { allowed: true, current: 200, limit: 500, warning: false, burst_zone: false, resets_at: '2026-02-24T05:00:00Z' },
        tokens: { allowed: true, current: 900000, limit: 2000000, warning: false, burst_zone: false, resets_at: '2026-02-24T05:00:00Z' },
        tool_calls: { allowed: true, current: 40, limit: 200, warning: false, burst_zone: false, resets_at: '2026-02-24T05:00:00Z' },
      },
      resources: {
        conversations: 5,
        max_conversations: 10,
        coaches: 2,
        max_coaches: 3,
      },
    },
    isLoading: false,
    error: null,
    level: 'none',
    sendDisabled: false,
    message: '',
    resetsAt: '',
    triggerCounter: null,
    invalidate: vi.fn(),
  }),
}));

// Default: API Tokens flag enabled so existing assertions still find the tab.
// Specific tests can re-mock with `vi.doMock` if they want the flag off.
vi.mock('../../hooks/useFeatureFlags', () => ({
  useFeatureFlags: () => ({
    flags: { api_tokens: true, billing_header: false },
    known: [],
    isLoading: false,
    isError: false,
  }),
  FEATURE_KEYS: { apiTokens: 'api_tokens', billingHeader: 'billing_header' },
}));

// Mock auth context
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: {
      id: 'user-1',
      email: 'test@pierre.dev',
      display_name: 'Test User',
      is_admin: false,
      role: 'user',
      tier: 'free',
      created_at: '2024-06-15T10:00:00Z',
    },
    logout: vi.fn(),
    isAuthenticated: true,
    isLoading: false,
  }),
}));

// OAuth spies shared with the Data Providers tests below.
const getAuthorizeUrlForProvider = vi.fn().mockResolvedValue('https://www.strava.com/oauth/authorize?x=1');
const getProvidersStatus = vi.fn().mockResolvedValue({ providers: [] });
const disconnectProvider = vi.fn().mockResolvedValue(undefined);

// Mock API service - factory must be self-contained (vi.mock is hoisted)
vi.mock('../../services/api', () => ({
  userApi: {
    getUserStats: vi.fn().mockResolvedValue({ connected_providers: 2, days_active: 45 }),
    getUserOAuthApps: vi.fn().mockResolvedValue({ apps: [] }),
    getMcpTokens: vi.fn().mockResolvedValue({ tokens: [] }),
    changePassword: vi.fn().mockResolvedValue({ message: 'Password changed successfully' }),
    updateProfile: vi.fn().mockResolvedValue({
      message: 'Profile updated',
      user: { id: 'user-1', email: 'test@pierre.dev', display_name: 'Test User' },
    }),
  },
  oauthApi: {
    getProvidersStatus: (...args: unknown[]) => getProvidersStatus(...args),
    getAuthorizeUrlForProvider: (...args: unknown[]) => getAuthorizeUrlForProvider(...args),
    disconnectProvider: (...args: unknown[]) => disconnectProvider(...args),
    disconnectIntervalsIcu: vi.fn().mockResolvedValue(undefined),
  },
  pierreApi: {
    adapter: {
      authStorage: {
        setUser: vi.fn().mockResolvedValue(undefined),
        getUser: vi.fn().mockResolvedValue(null),
        clear: vi.fn().mockResolvedValue(undefined),
      },
    },
    user: {
      getOAuthApps: vi.fn().mockResolvedValue({ apps: [] }),
      registerOAuthApp: vi.fn(),
      deleteOAuthApp: vi.fn(),
      createMcpToken: vi.fn(),
      revokeMcpToken: vi.fn(),
    },
  },
}));

// Render the Sciotte modal as a testid carrying its target so a fallback that
// opens it (target="strava") is observable.
vi.mock('../SciotteLoginModal', () => ({
  default: ({ isOpen, target }: { isOpen: boolean; target: string }) =>
    isOpen ? <div data-testid="sciotte-modal">{target}</div> : null,
}));

function renderUserSettings(props: Parameters<typeof UserSettings>[0] = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });

  return render(
    <QueryClientProvider client={queryClient}>
      <UserSettings {...props} />
    </QueryClientProvider>
  );
}

describe('UserSettings Component', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('Tab Navigation', () => {
    it('should render settings tab bar with all tabs', async () => {
      await act(async () => {
        renderUserSettings();
      });

      expect(screen.getByText('Profile')).toBeInTheDocument();
      // Data Providers moved out of Settings into a top-level sidebar tab.
      expect(screen.queryByText('Data Providers')).not.toBeInTheDocument();
      expect(screen.getByText('API Tokens')).toBeInTheDocument();
      expect(screen.getByText('AI Settings')).toBeInTheDocument();
      expect(screen.getByText('Messaging')).toBeInTheDocument();
      expect(screen.getByText('About')).toBeInTheDocument();
      expect(screen.getByText('Account')).toBeInTheDocument();
    });

    it('should start on Profile tab by default', async () => {
      await act(async () => {
        renderUserSettings();
      });

      // Profile tab should show user display name and email
      expect(screen.getByText('Test User')).toBeInTheDocument();
      expect(screen.getAllByText('test@pierre.dev').length).toBeGreaterThan(0);
    });

    it('should switch to About tab when clicked', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('About'));

      await waitFor(() => {
        expect(screen.getByText('Version')).toBeInTheDocument();
        expect(screen.getByText('1.0.0')).toBeInTheDocument();
      });
    });

    it('should switch to Account tab when clicked', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('Account'));

      await waitFor(() => {
        expect(screen.getByText('Danger Zone')).toBeInTheDocument();
      });
    });
  });

  describe('Profile Tab', () => {
    it('should display user avatar with gradient ring', async () => {
      await act(async () => {
        renderUserSettings();
      });

      // The avatar should be present with gradient border
      const avatarContainer = document.querySelector('.bg-gradient-to-br');
      expect(avatarContainer).toBeInTheDocument();
    });

    it('should display user stats', async () => {
      await act(async () => {
        renderUserSettings();
      });

      await waitFor(() => {
        expect(screen.getByText('Connected Providers')).toBeInTheDocument();
        expect(screen.getByText('Days Active')).toBeInTheDocument();
      });
    });

    it('should show display name input', async () => {
      await act(async () => {
        renderUserSettings();
      });

      const displayNameInput = screen.getByLabelText('Display Name');
      expect(displayNameInput).toBeInTheDocument();
      expect(displayNameInput).toHaveValue('Test User');
    });
  });

  describe('About Tab', () => {
    it('should show version, help, and terms links', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('About'));

      await waitFor(() => {
        expect(screen.getByText('Version')).toBeInTheDocument();
        expect(screen.getByText('1.0.0')).toBeInTheDocument();
        expect(screen.getByText('Help Center')).toBeInTheDocument();
        expect(screen.getByText('Terms & Privacy')).toBeInTheDocument();
      });
    });
  });

  describe('Account Tab', () => {
    it('should display member since date', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('Account'));

      await waitFor(() => {
        // The created_at is 2024-06-15 which formats to "Jun 15, 2024"
        expect(screen.getByText('Jun 15, 2024')).toBeInTheDocument();
      });
    });

    it('should show change password button', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('Account'));

      await waitFor(() => {
        expect(screen.getByText('Change Password')).toBeInTheDocument();
      });
    });

    it('should open change password modal', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('Account'));

      await waitFor(() => {
        expect(screen.getByText('Change Password')).toBeInTheDocument();
      });

      // Click the Change Password button
      const changePasswordButtons = screen.getAllByText('Change Password');
      // The button in Account tab (not the modal title)
      await user.click(changePasswordButtons[0]);

      await waitFor(() => {
        // Modal should appear with password fields
        expect(screen.getByLabelText('Current Password')).toBeInTheDocument();
        expect(screen.getByLabelText('New Password')).toBeInTheDocument();
        expect(screen.getByLabelText('Confirm New Password')).toBeInTheDocument();
      });
    });

    it('should validate passwords match in change password modal', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('Account'));

      await waitFor(() => {
        expect(screen.getByText('Change Password')).toBeInTheDocument();
      });

      // Open modal
      await user.click(screen.getByRole('button', { name: 'Change Password' }));

      await waitFor(() => {
        expect(screen.getByLabelText('Current Password')).toBeInTheDocument();
      });

      // Fill in mismatched passwords
      await user.type(screen.getByLabelText('Current Password'), 'password123');
      await user.type(screen.getByLabelText('New Password'), 'NewPass456');
      await user.type(screen.getByLabelText('Confirm New Password'), 'DifferentPass789');

      // The inline error should appear on the confirm field (no submit needed)
      await waitFor(() => {
        expect(screen.getByText(/passwords do not match/i)).toBeInTheDocument();
      });
    });

    it('should render usage card with quota meter labels', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('Account'));

      await waitFor(() => {
        expect(screen.getByText('Usage')).toBeInTheDocument();
        expect(screen.getByText('Daily Messages')).toBeInTheDocument();
        expect(screen.getByText('Daily Tokens')).toBeInTheDocument();
        expect(screen.getByText('Weekly Messages')).toBeInTheDocument();
      });
    });

    it('should display token counts in compact format', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('Account'));

      await waitFor(() => {
        // 145000 -> "145.0K" and 500000 -> "500.0K"
        expect(screen.getByText('145.0K / 500.0K')).toBeInTheDocument();
      });
    });

    it('should display resource counts for coaches and conversations', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('Account'));

      await waitFor(() => {
        expect(screen.getByText('Coaches')).toBeInTheDocument();
        expect(screen.getByText('2 / 3')).toBeInTheDocument();
        expect(screen.getByText('Conversations')).toBeInTheDocument();
        expect(screen.getByText('5 / 10')).toBeInTheDocument();
      });
    });

    it('should show sign out button in danger zone', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('Account'));

      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Sign Out' })).toBeInTheDocument();
      });
    });
  });

  describe('Data Providers (standalone top-level view)', () => {
    it('renders the providers panel via initialTab=connections + hideTabNav', async () => {
      await act(async () => {
        renderUserSettings({ initialTab: 'connections', hideTabNav: true });
      });

      await waitFor(() => {
        // Should show fitness providers and custom API credentials sections
        expect(screen.getByText('Fitness Providers')).toBeInTheDocument();
        expect(screen.getByText('Custom API Credentials')).toBeInTheDocument();
      });
    });
  });

  describe('Data Providers — Strava OAuth-first with Sciotte fallback', () => {
    const stravaCard = (recommended_backend: 'oauth' | 'mirror') => ({
      provider: 'sciotte',
      display_name: 'Strava',
      requires_oauth: false,
      connected: false,
      needs_reauth: false,
      capabilities: ['activities'],
      recommended_backend,
      seats_left: recommended_backend === 'oauth' ? 3 : 0,
    });

    beforeEach(() => {
      localStorage.clear();
      vi.stubGlobal('open', vi.fn().mockReturnValue({ closed: false, location: { href: '' } }));
    });

    it('launches Strava OAuth (not Sciotte) while seats remain', async () => {
      getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
      const user = userEvent.setup();
      await act(async () => {
        renderUserSettings({ initialTab: 'connections', hideTabNav: true });
      });

      const connect = await screen.findByRole('button', { name: 'Connect' });
      await user.click(connect);

      await waitFor(() => expect(getAuthorizeUrlForProvider).toHaveBeenCalledWith('strava'));
      expect(screen.queryByTestId('sciotte-modal')).not.toBeInTheDocument();
    });

    it('opens the Sciotte modal directly once the pool is exhausted', async () => {
      getProvidersStatus.mockResolvedValue({ providers: [stravaCard('mirror')] });
      const user = userEvent.setup();
      await act(async () => {
        renderUserSettings({ initialTab: 'connections', hideTabNav: true });
      });

      const connect = await screen.findByRole('button', { name: 'Connect' });
      await user.click(connect);

      expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('strava');
      expect(getAuthorizeUrlForProvider).not.toHaveBeenCalled();
    });

    it('renders a connected-but-dead session as "Reconnect needed", not "Connected"', async () => {
      // A connected provider whose scrape session died (needs_reauth) must show
      // the warning badge + a "Reconnect" affordance instead of a healthy pill.
      getProvidersStatus.mockResolvedValue({
        providers: [{ ...stravaCard('mirror'), connected: true, needs_reauth: true }],
      });
      await act(async () => {
        renderUserSettings({ initialTab: 'connections', hideTabNav: true });
      });

      expect(await screen.findByText('Reconnect needed')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument();
      // The healthy "Connected" badge must NOT be shown for a dead session.
      expect(screen.queryByText('Connected')).not.toBeInTheDocument();
    });

    // The failure-fallback path (a failed `pierre_oauth_result` reopening the
    // Sciotte modal) is exercised deterministically by the ProviderConnectionCards
    // and mobile ConnectionsScreen/OnboardingConnectScreen suites; UserSettings
    // routes it through the same handleConnectProvider poll, which cannot be
    // isolated reliably here because that poll's interval outlives unmount.
  });

  describe('Data Providers — Strava connected through the native OAuth row', () => {
    // Mirrors the live GET /api/providers payload for a Strava-connected user:
    // the grant lands on the native `strava` row while the `sciotte` row — the
    // card the UI actually renders — stays `connected: false`.
    const stravaOnNativeRow = () => [
      {
        provider: 'sciotte',
        display_name: 'Strava',
        requires_oauth: false,
        connected: false,
        needs_reauth: false,
        capabilities: ['activities'],
        recommended_backend: 'oauth' as const,
        seats_left: 7,
      },
      {
        provider: 'strava',
        display_name: 'Strava',
        requires_oauth: true,
        connected: true,
        needs_reauth: false,
        capabilities: ['activities'],
      },
    ];

    beforeEach(() => {
      localStorage.clear();
      vi.stubGlobal('open', vi.fn().mockReturnValue({ closed: false, location: { href: '' } }));
    });

    it('renders the single Strava card as Connected with a Disconnect control', async () => {
      getProvidersStatus.mockResolvedValue({ providers: stravaOnNativeRow() });

      await act(async () => {
        renderUserSettings({ initialTab: 'connections', hideTabNav: true });
      });

      expect(await screen.findByText('Connected')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Disconnect' })).toBeInTheDocument();
      // The connected card must not still offer "Connect"...
      expect(screen.queryByRole('button', { name: 'Connect' })).not.toBeInTheDocument();
      // ...and the native `strava` row must stay hidden, so exactly one Strava
      // card is on screen rather than a duplicate pair.
      expect(screen.getAllByText('Strava')).toHaveLength(1);
    });

    it('disconnects the native `strava` grant that actually backs the card', async () => {
      getProvidersStatus.mockResolvedValue({ providers: stravaOnNativeRow() });
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings({ initialTab: 'connections', hideTabNav: true });
      });

      await user.click(await screen.findByRole('button', { name: 'Disconnect' }));

      const dialog = await screen.findByRole('dialog');
      await user.click(within(dialog).getByRole('button', { name: 'Disconnect' }));

      // `DELETE /api/oauth/providers/{provider}/disconnect` deletes the tokens
      // for exactly the id it is handed, so `sciotte` here would leave the
      // Strava grant in place and the card connected after a refetch.
      await waitFor(() => expect(disconnectProvider).toHaveBeenCalledWith('strava'));
      expect(disconnectProvider).not.toHaveBeenCalledWith('sciotte');
    });

    it('carries needs_reauth from the native row instead of showing a healthy pill', async () => {
      const providers = stravaOnNativeRow();
      providers[1].needs_reauth = true;
      getProvidersStatus.mockResolvedValue({ providers });

      await act(async () => {
        renderUserSettings({ initialTab: 'connections', hideTabNav: true });
      });

      expect(await screen.findByText('Reconnect needed')).toBeInTheDocument();
      expect(screen.queryByText('Connected')).not.toBeInTheDocument();
    });

    it('guards the Strava/Sciotte switch when the mirror holds the connection', async () => {
      // The mirror (`sciotte`) holds a dead connection while shared OAuth seats
      // remain, so the card's Reconnect goes through native `strava` — the
      // exclusivity guard must catch that as a backend switch. It reads the raw
      // provider list, because the `strava` row it compares against is hidden.
      getProvidersStatus.mockResolvedValue({
        providers: [
          {
            provider: 'sciotte',
            display_name: 'Strava',
            requires_oauth: false,
            connected: true,
            needs_reauth: true,
            capabilities: ['activities'],
            recommended_backend: 'oauth' as const,
            seats_left: 7,
          },
        ],
      });
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings({ initialTab: 'connections', hideTabNav: true });
      });

      await user.click(await screen.findByRole('button', { name: 'Reconnect' }));

      expect(await screen.findByText('Switch Provider')).toBeInTheDocument();
      // The guard short-circuits the connect, so no OAuth URL is requested.
      expect(getAuthorizeUrlForProvider).not.toHaveBeenCalled();
    });
  });

  describe('API Tokens Tab', () => {
    it('should switch to tokens tab', async () => {
      const user = userEvent.setup();

      await act(async () => {
        renderUserSettings();
      });

      await user.click(screen.getByText('API Tokens'));

      await waitFor(() => {
        // Should show the tokens content with create button
        expect(screen.getByText('Create New Token')).toBeInTheDocument();
      });
    });
  });
});
