// ABOUTME: Comprehensive user settings with tabbed navigation
// ABOUTME: Includes Profile, Connections, Tokens, About, and Account sections
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useMemo, useEffect } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { format } from 'date-fns';
import { useAuth } from '../hooks/useAuth';
import { useTheme } from '../hooks/useTheme';
import { useTranslation } from '@pierre/i18n';
import { userApi, pierreApi, oauthApi } from '../services/api';
import type { ProviderStatus } from '../services/api';
import type { OAuthGrant, ThemePreference } from '@pierre/shared-types';
import {
  ADMIN_HIDDEN_PANES,
  APP_VERSION,
  HELP_URL,
  LEGAL_URL,
  settingsPaneSections,
} from '@pierre/shared-constants';
import { Button, Badge, ConfirmDialog, Input, Modal, ModalActions, Select, useErrorToast, Section, EmptyState } from './ui';
import { clsx } from 'clsx';
import A2AClientList from './A2AClientList';
import CreateA2AClient from './CreateA2AClient';
import CoachingPersonaTab from './CoachingPersonaTab';
import { SETTINGS_TABS, type SettingsTab } from './settings/settingsTabs';
import MessagingSettingsTab from './MessagingSettingsTab';
import NotificationSettingsTab from './NotificationSettingsTab';
import PrivacySettingsTab from './PrivacySettingsTab';
import MemoryPanel from './memory/MemoryPanel';
import { buildFitnessProviderCards } from '../utils/fitnessProviderCards';
import { QUERY_KEYS } from '../constants/queryKeys';
import { providerScopeLabelKey } from '@pierre/shared-constants';
import { useUsageStatus } from '../hooks/useUsageStatus';
import { useFeatureFlags, FEATURE_KEYS } from '../hooks/useFeatureFlags';
import SciotteLoginModal from './SciotteLoginModal';
import { LanguageSwitcher } from './LanguageSwitcher';
import IntervalsIcuLinkModal from './IntervalsIcuLinkModal';
import type { LimitCheckResult } from '../services/api/usage';

interface OAuthApp {
  provider: string;
  client_id: string;
  redirect_uri: string;
  created_at: string;
}

interface McpToken {
  id: string;
  name: string;
  token_prefix: string;
  expires_at: string | null;
  last_used_at: string | null;
  usage_count: number;
  is_revoked: boolean;
  created_at: string;
}

// BYO-OAuth-app credentials are only collected for WHOOP. Strava and Garmin
// use the Sciotte hosted-login flow (credentials handled in SciotteLoginModal)
// and have no developer-app registration step the user controls.
const PROVIDERS = [
  // WHOOP's brand black — a third-party colour, not a token (DESIGN.md §2).
  { id: 'whoop', name: 'WHOOP', color: 'text-on-surface' },
];

const MIN_PASSWORD_LENGTH = 8;

// Query key for the caller's connected OAuth apps (external MCP clients they
// approved on the consent screen). Kept local rather than in the shared
// QUERY_KEYS catalogue since this surface is web-only.
const CONNECTED_APPS_QUERY_KEY = ['user-connected-apps'] as const;

/** Format large numbers compactly (e.g. 145000 -> "145.0K", 2000000 -> "2.0M") */
function formatCompactNumber(value: number): string {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(1)}K`;
  }
  return value.toLocaleString();
}

/** Return Tailwind color class based on usage percentage: green < 70%, amber 70-90%, red > 90% */
function getUsageBarColor(current: number, limit: number): string {
  if (limit <= 0) return 'bg-activity';
  const pct = (current / limit) * 100;
  if (pct > 90) return 'bg-error';
  if (pct > 70) return 'bg-nutrition';
  return 'bg-activity';
}

/**
 * Format ISO 8601 reset time in the user's local timezone.
 *
 * `fallback` is the caller's translated wording for an unparseable timestamp:
 * this runs outside the component, so it cannot reach the catalogue itself.
 */
function formatResetTime(isoString: string, fallback: string): string {
  try {
    const date = new Date(isoString);
    return new Intl.DateTimeFormat(undefined, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(date);
  } catch {
    return fallback;
  }
}


export default function UserSettings({ initialTab = 'profile', hideTabNav = false }: { initialTab?: SettingsTab; hideTabNav?: boolean }) {
  const { user, logout, isAuthenticated } = useAuth();
  const { scheme, toggle: toggleTheme } = useTheme();
  const { t } = useTranslation();
  const showErrorToast = useErrorToast();
  const queryClient = useQueryClient();

  // Flip the theme locally, then tell the server which scheme was pinned so
  // the preference follows the athlete to their other devices. The write is
  // fire-and-forget: the local flip has already happened and must never be
  // undone or delayed by a failed request — failure only surfaces as a toast.
  const handleThemeToggle = () => {
    const next: ThemePreference = scheme === 'dark' ? 'light' : 'dark';
    toggleTheme();
    userApi.updateTheme(next).catch(() => {
      showErrorToast(t('settings.theme'), t('settings.themeSyncFailed'));
    });
  };
  const [activeTab, setActiveTab] = useState<SettingsTab>(initialTab);

  // Admin users don't need Data Providers, About, or Messaging tabs.
  // Gate on `role` (not is_admin) to stay consistent with Dashboard.tsx —
  // see migration 20260518000001 for the historic skew between the two.
  const isAdminUser = user?.role === 'admin' || user?.role === 'super_admin';
  const { flags: featureFlags } = useFeatureFlags();
  const visibleTabs = useMemo(() => {
    const base = isAdminUser
      ? SETTINGS_TABS.filter(tab => !ADMIN_HIDDEN_PANES.has(tab.id))
      : SETTINGS_TABS;
    // API Tokens tab is gated behind the per-tenant/per-user flag; default
    // off until an admin flips it on.
    if (!featureFlags[FEATURE_KEYS.apiTokens]) {
      return base.filter(tab => tab.id !== 'tokens');
    }
    return base;
  }, [isAdminUser, featureFlags]);

  // Snap activeTab back to a visible tab when the API Tokens flag flips off
  // while the user is sitting on that tab.
  useEffect(() => {
    if (!hideTabNav && !visibleTabs.some(tab => tab.id === activeTab)) {
      setActiveTab('profile');
    }
  }, [hideTabNav, visibleTabs, activeTab]);

  // Profile state
  const [displayName, setDisplayName] = useState(user?.display_name || '');
  const [isSaving, setIsSaving] = useState(false);
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

  // OAuth App state
  const [showAddCredentials, setShowAddCredentials] = useState(false);
  const [selectedProvider, setSelectedProvider] = useState('');
  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
  const [credentialMessage, setCredentialMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const [providerToDelete, setProviderToDelete] = useState<string | null>(null);

  // Token state
  const [tokenToRevoke, setTokenToRevoke] = useState<McpToken | null>(null);
  const [showCreateForm, setShowCreateForm] = useState(false);
  const [newTokenName, setNewTokenName] = useState('');
  const [expiresInDays, setExpiresInDays] = useState<number | undefined>(undefined);
  const [createdToken, setCreatedToken] = useState<{ token_value: string; name: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [showCreateA2AClient, setShowCreateA2AClient] = useState(false);
  const [showSetupInstructions, setShowSetupInstructions] = useState(false);

  // Fitness provider connection state
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);
  const [providerToDisconnect, setProviderToDisconnect] = useState<string | null>(null);
  const [providerMessage, setProviderMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const [sciotteModalTarget, setSciotteModalTarget] = useState<'strava' | 'garmin' | null>(null);
  const [intervalsModalOpen, setIntervalsModalOpen] = useState(false);
  const [providerConflict, setProviderConflict] = useState<{ connecting: string; disconnecting: string } | null>(null);

  // Connected OAuth apps (external MCP clients) revoke-confirmation target
  const [appToRevoke, setAppToRevoke] = useState<OAuthGrant | null>(null);

  // Change Password state
  const [showChangePassword, setShowChangePassword] = useState(false);
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [passwordMessage, setPasswordMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

  // Fetch fitness provider connection status
  const { data: providersResponse, isLoading: isLoadingProviders, refetch: refetchProviders } = useQuery({
    queryKey: QUERY_KEYS.user.providerConnections(),
    queryFn: () => oauthApi.getProvidersStatus(),
    enabled: isAuthenticated,
    // Always refetch when the Data Providers tab is opened — a provider connected
    // elsewhere (onboarding/chat OAuth, which uses a different query key) would
    // otherwise show stale t('app.connect') here until a manual reconnect.
    refetchOnMount: 'always',
  });

  // The raw list still carries the rows the cards hide (native `strava`,
  // `garmin`). The exclusivity guard below reads it, because the backend it
  // compares against is one of the hidden rows.
  const allProviders: ProviderStatus[] = providersResponse?.providers ?? [];
  const fitnessProviders = buildFitnessProviderCards(allProviders);

  // Fetch OAuth apps
  const { data: oauthAppsResponse, isLoading: isLoadingApps } = useQuery({
    queryKey: QUERY_KEYS.user.oauthApps(),
    queryFn: () => userApi.getOAuthApps(),
  });

  // Fetch user stats
  const { data: stats, isLoading: statsLoading } = useQuery({
    queryKey: QUERY_KEYS.user.stats(),
    queryFn: () => userApi.getStats(),
    staleTime: 30000,
  });

  // Fetch MCP tokens
  const { data: tokensResponse, isLoading: tokensLoading } = useQuery({
    queryKey: QUERY_KEYS.mcpTokens.list(),
    queryFn: () => userApi.getMcpTokens(),
    enabled: isAuthenticated,
  });

  // Fetch connected OAuth apps (external MCP clients the user approved on the
  // consent screen, e.g. Claude Desktop). Revoking one forces that client to
  // re-consent on its next authorization.
  const {
    data: connectedApps,
    isLoading: isLoadingConnectedApps,
    error: connectedAppsError,
  } = useQuery({
    queryKey: CONNECTED_APPS_QUERY_KEY,
    queryFn: () => oauthApi.listConnectedApps(),
    enabled: isAuthenticated,
  });

  // Fetch usage quota status
  const { data: usageData, isLoading: usageLoading } = useUsageStatus();

  // Which model answers this athlete. Read-only, and About is where it lives:
  // the athlete does not choose a provider, so the fact belongs beside the
  // version rather than beside a key field.
  const { data: llmSettings } = useQuery({
    queryKey: QUERY_KEYS.llmSettings.list(),
    queryFn: () => userApi.getLlmSettings(),
    enabled: isAuthenticated && activeTab === 'about',
  });
  const systemProvider = llmSettings?.system_provider;
  const coachModelLabel = systemProvider
    ? [systemProvider.display_name, systemProvider.model].filter(Boolean).join(' · ')
    : null;

  const oauthApps: OAuthApp[] = oauthAppsResponse?.apps || [];
  const tokens: McpToken[] = tokensResponse?.tokens || [];
  const activeTokens = tokens.filter((t) => !t.is_revoked);

  // Register OAuth app mutation
  const registerMutation = useMutation({
    mutationFn: (data: { provider: string; client_id: string; client_secret: string; redirect_uri: string }) =>
      userApi.registerOAuthApp(data),
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.oauthApps() });
      setCredentialMessage({ type: 'success', text: data.message });
      setShowAddCredentials(false);
      setSelectedProvider('');
      setClientId('');
      setClientSecret('');
    },
    onError: (error: Error) => {
      setCredentialMessage({ type: 'error', text: error.message || t('app.failedToSaveCredentials') });
    },
  });

  // Delete OAuth app mutation
  const deleteMutation = useMutation({
    mutationFn: (provider: string) => userApi.deleteOAuthApp(provider),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.oauthApps() });
      setCredentialMessage({ type: 'success', text: t('settingsErr.credentialsRemoved') });
      setProviderToDelete(null);
    },
    onError: (error: Error) => {
      setCredentialMessage({ type: 'error', text: error.message || t('app.failedRemoveCredentials') });
      setProviderToDelete(null);
    },
  });

  // Profile update mutation
  const profileMutation = useMutation({
    mutationFn: (data: { display_name: string }) => userApi.updateProfile(data),
    onSuccess: (response) => {
      setMessage({ type: 'success', text: response.message });
      pierreApi.adapter.authStorage.setUser(response.user);
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.all });
    },
    onError: (error: Error) => {
      setMessage({ type: 'error', text: error.message || t('app.failedToUpdateProfile') });
    },
    onSettled: () => {
      setIsSaving(false);
    },
  });

  // Token mutations
  const createTokenMutation = useMutation({
    mutationFn: (data: { name: string; expires_in_days?: number }) => userApi.createMcpToken(data),
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.mcpTokens.list() });
      setCreatedToken({ token_value: data.token_value ?? '', name: data.name });
      setShowCreateForm(false);
      setNewTokenName('');
      setExpiresInDays(undefined);
    },
  });

  const revokeTokenMutation = useMutation({
    mutationFn: (tokenId: string) => userApi.revokeMcpToken(tokenId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.mcpTokens.list() });
      setTokenToRevoke(null);
    },
  });

  // Revoke a connected OAuth app (external MCP client)
  const revokeConnectedAppMutation = useMutation({
    mutationFn: (id: string) => oauthApi.revokeConnectedApp(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: CONNECTED_APPS_QUERY_KEY });
      setAppToRevoke(null);
    },
  });

  // Change password mutation
  const changePasswordMutation = useMutation({
    mutationFn: (data: { current_password: string; new_password: string }) =>
      userApi.changePassword(data.current_password, data.new_password),
    onSuccess: () => {
      setPasswordMessage({ type: 'success', text: t('settingsErr.passwordChanged') });
      setCurrentPassword('');
      setNewPassword('');
      setConfirmPassword('');
      setTimeout(() => {
        setShowChangePassword(false);
        setPasswordMessage(null);
      }, 2000);
    },
    onError: (error: Error) => {
      setPasswordMessage({ type: 'error', text: error.message || t('app.failedChangePassword') });
    },
  });

  const handleSaveProfile = async () => {
    setIsSaving(true);
    setMessage(null);
    profileMutation.mutate({ display_name: displayName.trim() });
  };

  const handleAddCredentials = () => {
    if (!selectedProvider || !clientId.trim() || !clientSecret.trim()) {
      setCredentialMessage({ type: 'error', text: t('settingsErr.credentialsRequired') });
      return;
    }
    // Auto-generate the redirect URI from the current deployment
    const autoRedirectUri = `${window.location.origin}/api/oauth/callback/${selectedProvider}`;
    registerMutation.mutate({
      provider: selectedProvider,
      client_id: clientId.trim(),
      client_secret: clientSecret.trim(),
      redirect_uri: autoRedirectUri,
    });
  };

  const handleCreateToken = () => {
    if (!newTokenName.trim()) return;
    createTokenMutation.mutate({
      name: newTokenName.trim(),
      expires_in_days: expiresInDays,
    });
  };

  const handleChangePassword = () => {
    setPasswordMessage(null);
    if (!currentPassword || !newPassword || !confirmPassword) {
      setPasswordMessage({ type: 'error', text: t('settingsUi.allFieldsRequired') });
      return;
    }
    if (newPassword.length < MIN_PASSWORD_LENGTH) {
      setPasswordMessage({ type: 'error', text: t('frag.passwordMinError', { n: MIN_PASSWORD_LENGTH }) });
      return;
    }
    if (newPassword !== confirmPassword) {
      setPasswordMessage({ type: 'error', text: t('settingsErr.passwordMismatch') });
      return;
    }
    changePasswordMutation.mutate({
      current_password: currentPassword,
      new_password: newPassword,
    });
  };

  // Connect to a fitness provider via OAuth in a new tab
  // Check for Strava/Sciotte conflict before connecting. `backendId` is the
  // backend the connection will actually use, not the card that was clicked:
  // the `sciotte` card connects through native `strava` OAuth while shared
  // seats remain, and re-authing that same backend is not a conflict.
  const EXCLUSIVE_PROVIDERS = ['strava', 'sciotte'];
  const checkProviderConflict = (backendId: string): boolean => {
    if (!EXCLUSIVE_PROVIDERS.includes(backendId)) return false;
    const otherProvider = backendId === 'strava' ? 'sciotte' : 'strava';
    // Search the raw list: native `strava` is filtered out of the cards, so a
    // card-list lookup can never observe the Strava OAuth grant it guards.
    const otherConnected = allProviders.find(p => p.provider === otherProvider && p.connected);
    if (otherConnected) {
      setProviderConflict({
        connecting: backendId === 'sciotte' ? t('providers.stravaSciotte') : t('shell.sciotteTargetStrava'),
        disconnecting: otherProvider === 'sciotte' ? t('providers.stravaSciotte') : t('shell.sciotteTargetStrava'),
      });
      return true;
    }
    return false;
  };

  const handleConnectProvider = async (providerId: string, preopenedPopup?: Window | null) => {
    // Mobile Safari requires window.open to fire inside the synchronous
    // user-gesture call stack. Awaiting before window.open silently drops the
    // popup, leaving the spinner running until its 5-minute safety timeout.
    // Callers in flows that already burned the gesture (e.g. the Switch-Provider
    // dialog) pre-open a blank window and pass it in via preopenedPopup.
    const popup = preopenedPopup ?? window.open('about:blank', '_blank');

    try {
      setConnectingProvider(providerId);
      setProviderMessage(null);
      const authUrl = await oauthApi.getAuthorizeUrlForProvider(providerId);

      if (popup && !popup.closed) {
        popup.location.href = authUrl;
      } else {
        // Popup was blocked even when opened synchronously (strict mobile
        // Safari). Fall back to same-tab navigation. OAuthCallback writes
        // pierre_oauth_result to localStorage regardless of tab, so the
        // storage-event listener picks the result up when the user returns.
        window.location.href = authUrl;
        return;
      }

      // Listen for the OAuth callback result stored in localStorage by OAuthCallback
      const checkInterval = setInterval(() => {
        try {
          const resultStr = localStorage.getItem('pierre_oauth_result');
          if (resultStr) {
            const result = JSON.parse(resultStr);
            // Only process results less than 30 seconds old
            if (result.timestamp && Date.now() - result.timestamp < 30000 && result.provider === providerId) {
              localStorage.removeItem('pierre_oauth_result');
              clearInterval(checkInterval);
              setConnectingProvider(null);

              if (result.success) {
                setProviderMessage({ type: 'success', text: t('app.providerConnected', { provider: providerId }) });
                refetchProviders();
              } else if (providerId === 'strava') {
                // Strava OAuth failed (shared-app athlete cap actually exceeded
                // in a seat-count race, or the provider rejected the grant).
                // Fall back to the Sciotte credential login — same Strava data —
                // instead of leaving the user on an error message.
                setSciotteModalTarget('strava');
              } else {
                setProviderMessage({ type: 'error', text: t('frag.failedConnectProvider', { provider: providerId }) });
              }
            }
          }
        } catch {
          // Ignore localStorage parse errors
        }
      }, 500);

      // Safety timeout: stop checking after 5 minutes
      setTimeout(() => {
        clearInterval(checkInterval);
        setConnectingProvider(null);
      }, 300000);
    } catch (error) {
      if (popup && !popup.closed) {
        popup.close();
      }
      setConnectingProvider(null);
      // Couldn't start the Strava OAuth flow (init/network error, or the
      // platform Strava app is unconfigured). Fall back to the Sciotte
      // credential login rather than surfacing a dead-end error.
      if (providerId === 'strava') {
        setSciotteModalTarget('strava');
        return;
      }
      setProviderMessage({
        type: 'error',
        text: error instanceof Error ? error.message : t('settingsErr.startConnectionFailed'),
      });
    }
  };

  // Disconnect a fitness provider
  const handleDisconnectProvider = async (providerId: string) => {
    try {
      setProviderMessage(null);
      // Intervals.icu has its own (non-OAuth) disconnect endpoint.
      if (providerId === 'intervals_icu') {
        await oauthApi.disconnectIntervalsIcu();
      } else {
        await oauthApi.disconnectProvider(providerId);
      }
      setProviderToDisconnect(null);
      setProviderMessage({ type: 'success', text: `${providerId} disconnected` });
      refetchProviders();
    } catch (error) {
      setProviderToDisconnect(null);
      setProviderMessage({
        type: 'error',
        text: error instanceof Error ? error.message : t('settingsErr.disconnectFailed'),
      });
    }
  };

  // Display config for fitness providers (matching mobile). After the 2026-Q2
  // provider cleanup the API surfaces only sciotte / sciotte_garmin / whoop
  // (plus the synthetic dev providers); fitbit/coros/terra are feature-gated
  // off until we ship dedicated integrations. `strava` is retained so legacy
  // rows still display correctly when surfaced through the disconnect flow.
  const PROVIDER_DISPLAY: Record<string, { color: string; description: string }> = {
    strava: { color: '#FC4C02', description: t('providerBlurb.strava') },
    garmin: { color: '#007CC3', description: t('providerBlurb.garmin') },
    whoop: { color: 'currentColor', description: t('providerBlurb.whoop') },
    synthetic: { color: '#9C27B0', description: t('providerBlurb.synthTest') },
    synthetic_sleep: { color: '#673AB7', description: t('providerBlurb.synthSleep') },
    sciotte: { color: '#F97316', description: t('providerBlurb.strava') },
    sciotte_garmin: { color: '#007CC3', description: t('providerBlurb.garmin') },
    intervals_icu: { color: '#1273DE', description: t('providerBlurb.intervals') },
  };

  const copyToClipboard = async (text: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const getProviderInfo = (providerId: string) => {
    return PROVIDERS.find((p) => p.id === providerId) || { id: providerId, name: providerId, color: 'bg-surface-container-low' };
  };

  const configuredProviders = oauthApps.map((app) => app.provider);
  const availableProviders = PROVIDERS.filter((p) => !configuredProviders.includes(p.id));

  return (
    <div className="space-y-6">
      {/* Horizontal Tab Navigation. On mobile the strip overflows
          horizontally with scroll-snap so the active tab always lands at
          a clean offset; a right-edge gradient fade hints that more tabs
          live off-screen.
          From `md` it wraps instead of scrolling. Scrolling at desktop width
          silently clipped the last tab as soon as the labels stopped being
          English — French runs long enough that "Compte" fell off the edge —
          and a strip that scrolls inside its own box is invisible to the page
          gutter gate, which only measures the document. Wrapping cannot clip. */}
      {!hideTabNav && (
      <div className="relative border-b ghost-border">
        <nav
          className="flex gap-1 -mb-px overflow-x-auto scroll-smooth snap-x snap-mandatory md:flex-wrap md:overflow-x-visible md:snap-none"
          aria-label={t('settings.tabsLabel')}
        >
          {visibleTabs.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={clsx(
                'snap-start flex items-center gap-2 px-4 py-3 text-sm font-medium whitespace-nowrap transition-all duration-200 border-b-2 touch-target',
                activeTab === tab.id
                  ? 'border-primary text-on-surface'
                  : 'border-transparent text-on-surface-variant hover:text-on-surface hover:ghost-border'
              )}
            >
              <span className={clsx('flex-shrink-0', activeTab === tab.id ? 'text-primary' : '')}>{tab.icon}</span>
              {t(tab.nameKey)}
            </button>
          ))}
        </nav>
        {/* Right-edge fade: 24px linear gradient from transparent to surface,
            hides under tabs at >=md so only mobile sees it. */}
        <div
          aria-hidden="true"
          className="md:hidden pointer-events-none absolute top-0 bottom-0 right-0 w-6 bg-gradient-to-l from-surface to-transparent"
        />
      </div>
      )}

      {/* Settings Content */}
      <div className="space-y-10">
        {/* Profile Tab */}
        {activeTab === 'profile' && (
          <>
            <Section title={t('profile.title')}>
              <div className="space-y-4">
                {/* The identity row: a 40px initials avatar on the tint, the
                    name, the email, and the account status as a dot and a word. */}
                <div className="flex items-center gap-3 pb-4 border-b ghost-border-faint">
                  <span
                    aria-hidden="true"
                    className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-primary-container font-display text-sm font-semibold text-on-primary-container"
                  >
                    {(user?.display_name || user?.email)?.charAt(0).toUpperCase()}
                  </span>
                  <div className="min-w-0">
                    <p className="text-sm font-semibold text-on-surface break-words">{user?.display_name || t('app.noNameSet')}</p>
                    <p className="flex flex-wrap items-center gap-x-2 text-xs text-on-surface-variant">
                      <span className="break-all">{user?.email}</span>
                      <span className="inline-flex items-center gap-1.5">
                        <span
                          aria-hidden="true"
                          className={clsx('h-1.5 w-1.5 rounded-full', user?.user_status === 'active' ? 'bg-success' : 'bg-warning')}
                        />
                        {user?.user_status?.charAt(0).toUpperCase()}{user?.user_status?.slice(1)}
                      </span>
                    </p>
                  </div>
                </div>

                <Input
                  variant="dark"
                  label={t('profile.displayName')}
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  placeholder={t('profile.displayNamePlaceholder')}
                  size="lg"
                />

                <div>
                  <p className="block text-sm font-medium text-on-surface-variant mb-2">{t('common.email')}</p>
                  <p className="border-b border-outline-variant py-2 text-base text-on-surface-variant">{user?.email}</p>
                  <p className="text-xs text-outline mt-1.5">{t('profile.emailLocked')}</p>
                </div>

                {message && (
                  <div
                    className={`p-3 rounded-lg text-sm ${
                      message.type === 'success'
                        ? 'bg-activity/20 text-on-activity-container border border-activity/30'
                        : 'bg-error/20 text-error border border-error/30'
                    }`}
                  >
                    {message.text}
                  </div>
                )}

                <Button
                  variant="gradient"
                  onClick={handleSaveProfile}
                  loading={isSaving}
                  disabled={displayName === user?.display_name}
                  className=""
                >
                  {t('settings.saveChanges')}
                </Button>
              </div>
            </Section>

            {/* Appearance — the theme control's only reachable home once a
                user is signed in; the Login screen's toggle is gone by then. */}
            <Section title={t('settings.appearance')}>
              <div className="flex items-center justify-between gap-4">
                <div className="min-w-0">
                  <p className="font-medium text-on-surface">{t('settings.theme')}</p>
                  <p className="text-sm text-on-surface-variant">
                    {scheme === 'dark' ? t('settings.themeCurrentDark') : t('settings.themeCurrentLight')}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={handleThemeToggle}
                  aria-label={scheme === 'dark' ? t('settings.themeSwitchToLight') : t('settings.themeSwitchToDark')}
                  className="btn-base btn-outline gap-2"
                >
                  {scheme === 'dark' ? (
                    <svg className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth={1.75} viewBox="0 0 24 24" aria-hidden="true">
                      <circle cx="12" cy="12" r="4" />
                      <path strokeLinecap="round" d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M5.6 18.4l1.4-1.4M17 7l1.4-1.4" />
                    </svg>
                  ) : (
                    <svg className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth={1.75} viewBox="0 0 24 24" aria-hidden="true">
                      <path strokeLinecap="round" strokeLinejoin="round" d="M21 12.79A9 9 0 1111.21 3a7 7 0 009.79 9.79z" />
                    </svg>
                  )}
                  <span className="text-sm font-medium">
                    {scheme === 'dark' ? t('settings.themeSwitchToLight') : t('settings.themeSwitchToDark')}
                  </span>
                </button>
              </div>

              {/* Language — the switcher's only reachable home. It sets the
                  chrome language AND `users.locale`, so the coach answers in
                  the language the athlete reads the app in. */}
              <div className="mt-5 pt-5 border-t ghost-border-faint flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <p className="font-medium text-on-surface">{t('settings.language')}</p>
                  <p className="text-sm text-on-surface-variant">{t('settings.languageDescription')}</p>
                </div>
                <LanguageSwitcher serverLocale={user?.locale} />
              </div>
            </Section>

            {/* Two numbers, label over value, set apart by space — not two
                boxes with gradient text. */}
            <div className="flex gap-10">
              <div>
                <p className="text-xs text-outline">{t('providers.connectedTitle')}</p>
                <p className="mt-0.5 font-mono text-lg text-on-surface">{statsLoading ? '...' : (stats?.connected_providers ?? 0)}</p>
              </div>
              <div>
                <p className="text-xs text-outline">{t('profile.daysActive')}</p>
                <p className="mt-0.5 font-mono text-lg text-on-surface">{statsLoading ? '...' : (stats?.days_active ?? 0)}</p>
              </div>
            </div>
          </>
        )}

        {/* Connections Tab */}
        {activeTab === 'connections' && (
          <>
            {/* Fitness Providers - Connection Status */}
            <Section title={t('providers.fitnessTitle')} description={t('providers.fitnessHint')}>

              {providerMessage && (
                <div
                  className={clsx(
                    'p-3 rounded-lg text-sm mb-4',
                    providerMessage.type === 'success'
                      ? 'bg-activity/20 text-on-activity-container border border-activity/30'
                      : 'bg-error/20 text-error border border-error/30'
                  )}
                >
                  {providerMessage.text}
                </div>
              )}

              {isLoadingProviders ? (
                <div className="flex justify-center py-8">
                  <div className="pierre-spinner w-6 h-6"></div>
                </div>
              ) : fitnessProviders.length === 0 ? (
                <EmptyState>{t('providers.none')}</EmptyState>
              ) : (
                <div>
                  {fitnessProviders.map((provider) => {
                    const display = PROVIDER_DISPLAY[provider.provider] || {
                      color: '#607D8B',
                      description: t('providerBlurb.generic'),
                    };
                    const isConnecting = connectingProvider === provider.provider;

                    return (
                      <div
                        key={provider.provider}
                        className="border-t ghost-border-faint py-3 first:border-t-0 first:pt-0 last:pb-0"
                      >
                        <div className="flex items-center gap-3">
                          {/* The provider's colour lives in its letter, not in a tile. */}
                          <span
                            aria-hidden="true"
                            className="flex h-6 w-6 flex-shrink-0 items-center justify-center font-display text-sm font-bold"
                            style={{ color: display.color }}
                          >
                            {provider.display_name.charAt(0)}
                          </span>
                          <div className="flex-1 min-w-0">
                            <div className="flex items-center gap-2">
                              <p className="font-medium text-on-surface">{provider.display_name}</p>
                              {provider.connected && (
                                provider.needs_reauth ? (
                                  <span className="inline-flex items-center gap-1.5 text-xs text-on-surface-variant">
                                    <span aria-hidden="true" className="h-2 w-2 rounded-full bg-warning" />
                                    {t('providers.reconnectNeeded')}
                                  </span>
                                ) : (
                                  <span className="inline-flex items-center gap-1.5 text-xs text-on-surface-variant">
                                    <span aria-hidden="true" className="h-2 w-2 rounded-full bg-success" />
                                    {t('settingsUi.connected')}
                                  </span>
                                )
                              )}
                            </div>
                            <p className="text-xs text-on-surface-variant truncate">{display.description}</p>
                            {provider.capabilities.length > 0 && (
                              <p className="text-xs text-outline mt-0.5">
                                {provider.capabilities
                                  .map((scope) => {
                                    // A slug the catalogue has no word for prints as
                                    // itself — a provider that starts advertising a new
                                    // capability must not show a missing-key string.
                                    const key = providerScopeLabelKey(scope);
                                    return key ? t(key) : scope;
                                  })
                                  .join(', ')}
                              </p>
                            )}
                          </div>
                          <div className="flex-shrink-0">
                            {/* A connected-but-dead session (needs_reauth) falls through to the
                                connect flow below, relabelled t('app.reconnect'), so the user can revive
                                it rather than being stuck looking at a healthy-seeming row. */}
                            {provider.connected && !provider.needs_reauth ? (
                              (provider.requires_oauth || provider.provider.startsWith('sciotte') || provider.provider === 'intervals_icu') && (
                                <Button
                                  variant="tertiary"
                                  size="sm"
                                  onClick={() => setProviderToDisconnect(provider.connectionProvider)}
                                  className="text-error"
                                >
                                  {t('settingsUi.disconnect')}
                                </Button>
                              )
                            ) : provider.provider === 'intervals_icu' ? (
                              <Button
                                variant="outline"
                                size="sm"
                                onClick={() => setIntervalsModalOpen(true)}
                              >
                                {provider.needs_reauth ? t('settingsUi.reconnect') : t('shell.intervalsConnectAction')}
                              </Button>
                            ) : provider.provider === 'sciotte' || provider.provider === 'sciotte_garmin' ? (
                              <Button
                                variant="outline"
                                size="sm"
                                onClick={() => {
                                  // The `sciotte` card is the user-facing t('app.brandStrava') card. OAuth is
                                  // the default while shared-app seats remain (server recommends
                                  // `oauth`); once the athlete cap is reached it recommends `mirror`
                                  // and we open the Sciotte credential login. If the OAuth attempt
                                  // itself fails, handleConnectProvider falls back to Sciotte.
                                  // Garmin (`sciotte_garmin`) is always the credential flow.
                                  const backend =
                                    provider.provider === 'sciotte' && provider.recommended_backend === 'oauth'
                                      ? 'strava'
                                      : provider.provider;
                                  // Guard on the resolved backend so re-authing the backend that
                                  // already holds the grant isn't mistaken for a provider switch.
                                  if (checkProviderConflict(backend)) return;
                                  if (backend === 'strava') {
                                    void handleConnectProvider('strava');
                                  } else {
                                    setSciotteModalTarget(provider.provider === 'sciotte_garmin' ? 'garmin' : 'strava');
                                  }
                                }}
                              >
                                {provider.needs_reauth ? t('settingsUi.reconnect') : t('shell.intervalsConnectAction')}
                              </Button>
                            ) : provider.requires_oauth ? (
                              <Button
                                variant="outline"
                                size="sm"
                                onClick={() => {
                                  if (!checkProviderConflict(provider.provider)) handleConnectProvider(provider.provider);
                                }}
                                loading={isConnecting}
                              >
                                {provider.needs_reauth ? t('settingsUi.reconnect') : t('shell.intervalsConnectAction')}
                              </Button>
                            ) : (
                              <Badge variant="secondary">{t('settingsUi.manual')}</Badge>
                            )}
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}

              {/* Privacy note */}
              <p className="mt-4 text-xs text-outline">{t('providers.privacyNote')}</p>
            </Section>

            {/* OAuth App Credentials (Advanced) */}
            <Section
              title={t('credentials.title')}
              description={t('credentials.useOwnHint')}
              actions={
                availableProviders.length > 0 ? (
                  <Button variant="tertiary" size="sm" onClick={() => setShowAddCredentials(true)}>
                    {t('providers.add')}
                  </Button>
                ) : null
              }
            >

            {credentialMessage && (
              <div
                className={`p-3 rounded-lg text-sm mb-4 ${
                  credentialMessage.type === 'success'
                    ? 'bg-activity/20 text-on-activity-container border border-activity/30'
                    : 'bg-error/20 text-error border border-error/30'
                }`}
              >
                {credentialMessage.text}
              </div>
            )}

            {isLoadingApps ? (
              <div className="flex justify-center py-6">
                <div className="pierre-spinner w-6 h-6"></div>
              </div>
            ) : oauthApps.length === 0 ? (
              <EmptyState>
                <span>{t('credentials.empty')}</span> <span>{t('credentials.addHint')}</span>
              </EmptyState>
            ) : (
              <div>
                {oauthApps.map((app) => {
                  const provider = getProviderInfo(app.provider);
                  return (
                    <div key={app.provider} className="flex items-center justify-between border-t ghost-border-faint py-3 first:border-t-0">
                      <div className="flex items-center gap-3">
                        <span aria-hidden="true" className={`flex h-6 w-6 items-center justify-center font-display text-sm font-bold ${provider.color}`}>
                          {provider.name.charAt(0)}
                        </span>
                        <div>
                          <p className="font-medium text-on-surface">{provider.name}</p>
                          <p className="text-xs text-outline">{t('frag.clientIdLabel')} {app.client_id.substring(0, 8)}...</p>
                        </div>
                      </div>
                      <div className="flex items-center gap-2">
                        <Badge variant="success">{t('settingsUi.configured')}</Badge>
                        <Button variant="danger" size="sm" onClick={() => setProviderToDelete(app.provider)}>
                          {t('settingsUi.remove')}
                        </Button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}

            {/* Add Credentials Modal */}
            <Modal
              isOpen={showAddCredentials}
              onClose={() => {
                setShowAddCredentials(false);
                setSelectedProvider('');
                setClientId('');
                setClientSecret('');
                setCredentialMessage(null);
              }}
              title={t('credentials.add')}
              size="md"
              footer={
                <div className="flex gap-2 justify-end">
                  <Button
                    variant="secondary"
                    onClick={() => {
                      setShowAddCredentials(false);
                      setSelectedProvider('');
                      setClientId('');
                      setClientSecret('');
                      setCredentialMessage(null);
                    }}
                  >
                    {t('settingsUi.cancel')}
                  </Button>
                  <Button
                    variant="gradient"
                    onClick={handleAddCredentials}
                    loading={registerMutation.isPending}
                    disabled={!selectedProvider || !clientId || !clientSecret}
                  >
                    {t('credentials.save')}
                  </Button>
                </div>
              }
            >
              <p className="text-sm text-on-surface-variant mb-5">{t('credentials.useOwnAppHint')}</p>

              {credentialMessage && (
                <div className={`p-3 rounded-lg text-sm mb-4 ${
                  credentialMessage.type === 'success'
                    ? 'bg-activity/20 text-on-activity-container border border-activity/30'
                    : 'bg-error/20 text-error border border-error/30'
                }`}>
                  {credentialMessage.text}
                </div>
              )}

              <div className="space-y-4">
                <div>
                  <Select
                    label={t('providers.provider')}
                    value={selectedProvider}
                    onChange={(e) => setSelectedProvider(e.target.value)}
                    placeholder={t('providers.select')}
                    options={availableProviders.map((provider) => ({
                      value: provider.id,
                      label: provider.name,
                    }))}
                  />
                </div>

                <Input
                  variant="dark"
                  label={t('credentials.clientId')}
                  value={clientId}
                  onChange={(e) => setClientId(e.target.value)}
                  placeholder={t('credentials.clientIdPlaceholder')}
                />

                <Input
                  variant="dark"
                  label={t('credentials.clientSecret')}
                  type="password"
                  value={clientSecret}
                  onChange={(e) => setClientSecret(e.target.value)}
                  placeholder={t('credentials.clientSecretPlaceholder')}
                />

                {selectedProvider && (
                  <div className="text-xs text-outline space-y-1">
                    <p>{t('frag.inYour')} {selectedProvider} app settings, set:</p>
                    <p>{t('settingsUi.callbackDomain')} <code className="text-on-surface-variant">{window.location.host}</code></p>
                  </div>
                )}
              </div>
            </Modal>
          </Section>
          </>
        )}

        {/* Tokens Tab */}
        {activeTab === 'tokens' && (
          <>
            {/* Created Token Display */}
            {createdToken && (
              <div className="bg-success/10 border border-success/30 rounded-lg p-6">
                <div className="flex items-start gap-3">
                  <svg className="w-6 h-6 text-success mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"
                    />
                  </svg>
                  <div className="flex-1">
                    <h3 className="text-lg font-medium text-success">{t('frag.tokenCreatedLabel')} {createdToken.name}</h3>
                    <p className="text-success/80 mt-1 mb-3">{t('settingsUi.copyTokenNow')}</p>
                    <div className="flex items-center gap-2">
                      <code className="flex-1 px-3 py-2 bg-surface-container-low border border-success/30 rounded font-mono text-sm break-all text-on-surface">
                        {createdToken.token_value}
                      </code>
                      <Button onClick={() => copyToClipboard(createdToken.token_value)} variant="secondary" size="sm">
                        {copied ? t('settingsUi.copied') : t('settingsUi.copy')}
                      </Button>
                    </div>
                    <Button onClick={() => setCreatedToken(null)} variant="secondary" size="sm" className="mt-3">
                      {t('settingsUi.dismiss')}
                    </Button>
                  </div>
                </div>
              </div>
            )}

            <Section title={t('tokens.title')} description={t('tokens.activeCount', { count: activeTokens.length })}>

              {/* Create Token Section */}
              <div className="mb-6">
                {!showCreateForm ? (
                  <Button onClick={() => setShowCreateForm(true)} variant="primary">
                    {t('tokens.createNew')}
                  </Button>
                ) : (
                  <div className="bg-surface-container-low border ghost-border rounded-lg p-4 space-y-4">
                    <h4 className="font-medium text-on-surface">{t('tokens.create')}</h4>
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                      <Input
                        variant="dark"
                        label={t('tokens.name')}
                        value={newTokenName}
                        onChange={(e) => setNewTokenName(e.target.value)}
                        placeholder="e.g., Claude Desktop, Cursor IDE"
                      />
                      <Select
                        label={t('tokens.expiresInDays')}
                        value={expiresInDays || ''}
                        onChange={(e) => setExpiresInDays(e.target.value ? Number(e.target.value) : undefined)}
                        options={[
                          { value: '', label: t('settingsUi.neverExpires') },
                          { value: '30', label: '30 days' },
                          { value: '90', label: '90 days' },
                          { value: '180', label: '180 days' },
                          { value: '365', label: '1 year' },
                        ]}
                      />
                    </div>
                    <div className="flex gap-2">
                      <Button
                        onClick={handleCreateToken}
                        disabled={!newTokenName.trim() || createTokenMutation.isPending}
                        variant="primary"
                      >
                        {createTokenMutation.isPending ? t('tokens.creating') : t('tokens.createAction')}
                      </Button>
                      <Button onClick={() => setShowCreateForm(false)} variant="secondary">
                        {t('settingsUi.cancel')}
                      </Button>
                    </div>
                  </div>
                )}
              </div>

              {/* Token List */}
              {tokensLoading ? (
                <div className="flex justify-center py-8">
                  <div className="pierre-spinner w-8 h-8"></div>
                </div>
              ) : tokens.length === 0 ? (
                <EmptyState>
                  <span>{t('tokens.empty')}</span> <span>{t('tokens.createHint')}</span>
                </EmptyState>
              ) : (
                <div>
                  {tokens.map((token) => (
                    <div key={token.id} className="border-t ghost-border-faint py-4 first:border-t-0">
                      <div className="flex items-start justify-between">
                        <div className="flex-1">
                          <div className="flex items-center gap-2">
                            <h3 className="font-sans text-sm font-semibold tracking-normal text-on-surface">{token.name}</h3>
                            <Badge variant={token.is_revoked ? 'info' : 'success'}>
                              {token.is_revoked ? t('settingsUi.revoked') : t('tokens.statusActive')}
                            </Badge>
                          </div>
                          <code className="inline-flex items-center gap-1 mt-1 px-2 py-0.5 bg-surface-container-high text-on-surface text-xs font-mono rounded border ghost-border">
                            {token.token_prefix}...
                          </code>
                          <div className="mt-4 grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
                            <div>
                              <span className="text-outline">{t('settingsUi.createdLabel')}</span>
                              <p className="font-medium text-on-surface">{format(new Date(token.created_at), 'MMM d, yyyy')}</p>
                            </div>
                            <div>
                              <span className="text-outline">{t('settingsUi.expiresLabel')}</span>
                              <p className="font-medium text-on-surface">
                                {token.expires_at ? format(new Date(token.expires_at), 'MMM d, yyyy') : t('settingsUi.neverValue')}
                              </p>
                            </div>
                            <div>
                              <span className="text-outline">{t('settingsUi.usageLabel')}</span>
                              <p className="font-medium text-on-surface">{token.usage_count} requests</p>
                            </div>
                            <div>
                              <span className="text-outline">{t('settingsUi.lastUsedLabel')}</span>
                              <p className="font-medium text-on-surface">
                                {token.last_used_at ? format(new Date(token.last_used_at), 'MMM d, yyyy') : t('settingsUi.neverValue')}
                              </p>
                            </div>
                          </div>
                        </div>
                        {!token.is_revoked && (
                          <Button
                            onClick={() => setTokenToRevoke(token)}
                            disabled={revokeTokenMutation.isPending}
                            variant="secondary"
                            className="text-error hover:bg-error/20"
                            size="sm"
                          >
                            {t('settingsUi.revoke')}
                          </Button>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              )}

              {/* Setup Instructions - Collapsible */}
              <div className="border-t ghost-border mt-6 pt-4">
                <button
                  onClick={() => setShowSetupInstructions(!showSetupInstructions)}
                  className="flex items-center justify-between w-full text-left"
                >
                  <div className="flex items-center gap-2">
                    <svg className="w-5 h-5 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                      />
                    </svg>
                    <span className="font-medium text-on-surface">{t('tokens.setupInstructions')}</span>
                    <span className="text-sm text-on-surface-variant">{t('tokens.setupInstructionsFor')}</span>
                  </div>
                  <svg
                    className={`w-5 h-5 text-outline transition-transform ${showSetupInstructions ? 'rotate-180' : ''}`}
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                  </svg>
                </button>

                {showSetupInstructions && (
                  <div className="mt-4 space-y-4">
                    <div className="bg-surface-container-low border ghost-border rounded-lg p-4">
                      <h4 className="font-medium text-on-surface mb-2">{t('settings.claudeDesktop')}</h4>
                      <p className="text-sm text-on-surface-variant mb-3">
                        {t('settingsUi.claudeConfigHint')}
                      </p>
                      <pre className="text-xs bg-surface-container-low text-on-surface p-3 rounded overflow-x-auto border ghost-border">
                        {`{
  "mcpServers": {
    "pierre": {
      "command": "npx",
      "args": ["-y", "@anthropic/mcp-client"],
      "env": {
        "MCP_SERVER_URL": "${window.location.origin}/mcp",
        "MCP_TOKEN": "<your-token-here>"
      }
    }
  }
}`}
                      </pre>
                    </div>

                    <div className="bg-surface-container-low border ghost-border rounded-lg p-4">
                      <h4 className="font-medium text-on-surface mb-2">ChatGPT</h4>
                      <p className="text-sm text-on-surface-variant mb-3">{t('settingsUi.chatgptConfigHint')}</p>
                      <pre className="text-xs bg-surface-container-low text-on-surface p-3 rounded overflow-x-auto border ghost-border">
                        {`Server URL: ${window.location.origin}/mcp
Authorization: Bearer <your-token-here>`}
                      </pre>
                    </div>
                  </div>
                )}
              </div>
            </Section>

            {/* Connected Apps Section */}
            <Section title={t('tokens.connectedApps')} description={t('providers.thirdPartyHint')}>
              {showCreateA2AClient ? (
                <CreateA2AClient
                  onSuccess={() => setShowCreateA2AClient(false)}
                  onCancel={() => setShowCreateA2AClient(false)}
                />
              ) : (
                <A2AClientList onCreateClient={() => setShowCreateA2AClient(true)} />
              )}
            </Section>
          </>
        )}

        {activeTab === 'coaching' && <CoachingPersonaTab />}

        {activeTab === 'messaging' && <MessagingSettingsTab />}

        {activeTab === 'notifications' && <NotificationSettingsTab />}

        {activeTab === 'memory' && <MemoryPanel />}

        {activeTab === 'privacy' && <PrivacySettingsTab />}

        {/* About Tab — rows in the order the shared pane declaration holds
            them, so the phone's About screen lists the same four things. */}
        {activeTab === 'about' && (
          <Section title={t('about.title')}>
            <div>
              {settingsPaneSections('about').map((section) => {
                switch (section) {
                  case 'version':
                    return (
                      <div
                        key={section}
                        data-testid="about-section-version"
                        className="flex items-center gap-3 border-t ghost-border-faint py-3 first:border-t-0"
                      >
                        <div className="flex-1">
                          <p className="text-xs text-on-surface-variant">{t('settingsUi.version')}</p>
                          <p className="text-sm text-on-surface font-medium">{APP_VERSION}</p>
                        </div>
                      </div>
                    );
                  case 'coach-model':
                    return (
                      <div
                        key={section}
                        data-testid="about-section-coach-model"
                        className="flex items-center gap-3 border-t ghost-border-faint py-3 first:border-t-0"
                      >
                        <div className="flex-1 min-w-0">
                          <p className="text-xs text-on-surface-variant">{t('about.coachModel')}</p>
                          <p className="text-on-surface font-medium break-words" data-testid="about-coach-model-value">
                            {coachModelLabel ?? t('about.coachModelUnknown')}
                          </p>
                        </div>
                      </div>
                    );
                  case 'help':
                    return (
                      <a
                        key={section}
                        data-testid="about-section-help"
                        href={HELP_URL}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="group flex items-center gap-3 border-t ghost-border-faint py-3 first:border-t-0 transition-colors hover:text-primary"
                      >
                        <div className="flex-1">
                          <p className="text-on-surface font-medium">{t('about.helpCenter')}</p>
                          <p className="text-xs text-on-surface-variant">{t('about.helpHint')}</p>
                        </div>
                        <svg className="w-5 h-5 text-outline group-hover:text-on-surface transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                        </svg>
                      </a>
                    );
                  case 'legal':
                    return (
                      <a
                        key={section}
                        data-testid="about-section-legal"
                        href={LEGAL_URL}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="group flex items-center gap-3 border-t ghost-border-faint py-3 first:border-t-0 transition-colors hover:text-primary"
                      >
                        <div className="flex-1">
                          <p className="text-on-surface font-medium">{t('about.legalDocuments')}</p>
                          <p className="text-xs text-on-surface-variant">{t('about.legalHint')}</p>
                        </div>
                        <svg className="w-5 h-5 text-outline group-hover:text-on-surface transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                        </svg>
                      </a>
                    );
                  default:
                    return null;
                }
              })}
            </div>
          </Section>
        )}

        {/* Account Tab — the cards in the order the shared pane declaration
            holds them, so the phone groups the same five things under Account
            rather than scattering usage and connected apps across a scroll. */}
        {activeTab === 'account' && (
          <>
            {settingsPaneSections('account').map((section) => {
              switch (section) {
                case 'account-status':
                  return (
                      <div key={section} data-testid="account-section-account-status">
                        <Section title={t('profile.accountStatus')}>
                        <div className="space-y-3">
                          <div className="flex justify-between items-center py-2 border-b ghost-border">
                            <span className="text-on-surface-variant">{t('settingsUi.status')}</span>
                            <span className="inline-flex items-center gap-1.5 text-sm text-on-surface">
                              <span
                                aria-hidden="true"
                                className={`h-1.5 w-1.5 rounded-full ${user?.user_status === 'active' ? 'bg-success' : 'bg-warning'}`}
                              />
                              {user?.user_status?.charAt(0).toUpperCase()}
                              {user?.user_status?.slice(1)}
                            </span>
                          </div>
                          <div className="flex justify-between items-center py-2 border-b ghost-border">
                            <span className="text-on-surface-variant">{t('settingsUi.role')}</span>
                            <span className="text-on-surface capitalize">{user?.role}</span>
                          </div>
                          <div className="flex justify-between items-center py-2">
                            <span className="text-on-surface-variant">{t('profile.memberSince')}</span>
                            <span className="text-on-surface">
                              {user?.created_at
                                ? format(new Date(user.created_at), 'MMM d, yyyy')
                                : t('settingsUi.unknownDate')}
                            </span>
                          </div>
                        </div>
                        </Section>
                      </div>
                  );
                case 'usage':
                  // Usage quotas are user-facing; an operator has none of their own.
                  return isAdminUser ? null : (
                      <div key={section} data-testid="account-section-usage">
                        <Section title={t('settingsUi.usage')} description={t('account.quotaHint')}>

                        {usageLoading ? (
                          <div className="flex justify-center py-6">
                            <div className="pierre-spinner w-6 h-6"></div>
                          </div>
                        ) : !usageData ? (
                          <p className="text-sm text-outline text-center py-4">{t('account.usageUnavailable')}</p>
                        ) : (
                          <div className="space-y-5">
                            {/* Progress bars */}
                            <div className="space-y-4">
                              {([
                                { label: t('settingsUi.dailyMessages'), counter: usageData.daily.messages },
                                { label: t('settingsUi.dailyTokens'), counter: usageData.daily.tokens, compact: true },
                                { label: t('settingsUi.weeklyMessages'), counter: usageData.weekly.messages },
                              ] as { label: string; counter: LimitCheckResult; compact?: boolean }[]).map(({ label, counter, compact }) => {
                                const pct = counter.limit > 0 ? Math.min((counter.current / counter.limit) * 100, 100) : 0;
                                return (
                                  <div key={label}>
                                    <div className="flex justify-between items-center mb-1.5">
                                      <span className="text-sm font-medium text-on-surface">{label}</span>
                                      <span className="text-sm text-on-surface-variant">
                                        {compact ? formatCompactNumber(counter.current) : counter.current.toLocaleString()}
                                        {' / '}
                                        {compact ? formatCompactNumber(counter.limit) : counter.limit.toLocaleString()}
                                      </span>
                                    </div>
                                    <div className="h-2 bg-surface-container-high rounded-full overflow-hidden">
                                      <div
                                        className={clsx(
                                          'h-full rounded-full transition-all duration-300',
                                          getUsageBarColor(counter.current, counter.limit),
                                        )}
                                        style={{ width: `${pct}%` }}
                                      />
                                    </div>
                                  </div>
                                );
                              })}
                            </div>

                            {/* Reset time */}
                            <p className="text-xs text-outline">
                              {t('frag.dailyLimitsResetAt')}{' '}
                              {formatResetTime(usageData.daily.messages.resets_at, t('settingsUi.midnightUtc'))}
                            </p>

                            {/* Resource counts (user-facing only, not shown for admin) */}
                            {!isAdminUser && (
                            <div className="border-t ghost-border pt-4">
                              <div className="grid grid-cols-2 gap-4">
                                <div>
                                  <p className="text-xs text-outline mb-1">{t('settingsUi.coaches')}</p>
                                  <p className="text-sm font-medium text-on-surface">
                                    {usageData.resources.coaches} / {usageData.resources.max_coaches}
                                  </p>
                                </div>
                                <div>
                                  <p className="text-xs text-outline mb-1">{t('settingsUi.conversations')}</p>
                                  <p className="text-sm font-medium text-on-surface">
                                    {usageData.resources.conversations} / {usageData.resources.max_conversations}
                                  </p>
                                </div>
                              </div>
                            </div>
                            )}
                          </div>
                        )}
                        </Section>
                      </div>
                  );
                case 'security':
                  return (
                      <div key={section} data-testid="account-section-security">
                        <Section title={t('settingsUi.security')}>
                        <div className="space-y-4">
                          <div>
                            <h3 className="font-medium text-on-surface mb-2">{t('settingsUi.password')}</h3>
                            <p className="text-sm text-on-surface-variant mb-3">{t('password.changeHint')}</p>
                            <Button variant="outline" size="sm" onClick={() => setShowChangePassword(true)}>
                              {t('password.change')}
                            </Button>
                          </div>
                        </div>
                        </Section>
                      </div>
                  );
                case 'connected-mcp-apps':
                  // External OAuth clients (e.g. Claude Desktop) the user approved on
                  // the consent screen. Distinct from the A2A "Connected Apps" card in
                  // the API Tokens pane, which lists self-registered agent clients.
                  return (
                      <div key={section} data-testid="account-section-connected-mcp-apps">
                        <Section title={t('tokens.connectedMcpApps')} description={t('tokens.connectedAppsHint')}>

                        {isLoadingConnectedApps ? (
                          <div className="flex justify-center py-8">
                            <div className="pierre-spinner w-6 h-6"></div>
                          </div>
                        ) : connectedAppsError ? (
                          <div className="p-3 rounded-lg text-sm bg-error/20 text-error border border-error/30">
                            {connectedAppsError instanceof Error
                              ? connectedAppsError.message
                              : t('settingsErr.loadAppsFailed')}
                          </div>
                        ) : connectedApps && connectedApps.length > 0 ? (
                          <div>
                            {connectedApps.map((app) => (
                              <div
                                key={app.id}
                                className="flex items-start justify-between gap-3 border-t ghost-border-faint py-3 first:border-t-0"
                              >
                                <div className="min-w-0">
                                  <p className="font-medium text-on-surface break-all">{app.client_id}</p>
                                  <p className="text-sm text-on-surface-variant break-words">{app.scope}</p>
                                  <p className="text-xs text-outline mt-1">
                                    {t('frag.connected')} {format(new Date(app.granted_at), 'MMM d, yyyy')}
                                  </p>
                                </div>
                                <Button
                                  variant="secondary"
                                  size="sm"
                                  onClick={() => setAppToRevoke(app)}
                                  disabled={revokeConnectedAppMutation.isPending}
                                  className="flex-shrink-0 text-error hover:bg-error/20"
                                >
                                  {t('settingsUi.revoke')}
                                </Button>
                              </div>
                            ))}
                          </div>
                        ) : (
                          <EmptyState>{t('tokens.connectedAppsEmpty')}</EmptyState>
                        )}
                        </Section>
                      </div>
                  );
                case 'sign-out':
                  return (
                      <div key={section} data-testid="account-section-sign-out">
                        <Section title={t('account.dangerZone')}>
                        <div className="space-y-4">
                          <div>
                            <h3 className="font-medium text-on-surface mb-2">{t('account.signOut')}</h3>
                            <p className="text-sm text-on-surface-variant mb-3">{t('account.signOutHint')}</p>
                            <Button variant="secondary" size="sm" onClick={logout}>
                              {t('account.signOut')}
                            </Button>
                          </div>
                        </div>
                        </Section>
                      </div>
                  );
                default:
                  return null;
              }
            })}
          </>
        )}
      </div>

      {/* Change Password Modal */}
      <Modal
        isOpen={showChangePassword}
        onClose={() => {
          setShowChangePassword(false);
          setCurrentPassword('');
          setNewPassword('');
          setConfirmPassword('');
          setPasswordMessage(null);
        }}
        title={t('password.change')}
        size="sm"
        footer={
          <ModalActions>
            <Button
              variant="secondary"
              onClick={() => {
                setShowChangePassword(false);
                setCurrentPassword('');
                setNewPassword('');
                setConfirmPassword('');
                setPasswordMessage(null);
              }}
            >
              {t('settingsUi.cancel')}
            </Button>
            <Button
              variant="gradient"
              onClick={handleChangePassword}
              loading={changePasswordMutation.isPending}
              disabled={!currentPassword || !newPassword || !confirmPassword}
            >
              {t('password.update')}
            </Button>
          </ModalActions>
        }
      >
        <div className="space-y-4">
          {passwordMessage && (
            <div
              className={`p-3 rounded-lg text-sm ${
                passwordMessage.type === 'success'
                  ? 'bg-activity/20 text-on-activity-container border border-activity/30'
                  : 'bg-error/20 text-error border border-error/30'
              }`}
            >
              {passwordMessage.text}
            </div>
          )}
          <Input
            variant="dark"
            label={t('password.current')}
            type="password"
            value={currentPassword}
            onChange={(e) => setCurrentPassword(e.target.value)}
            placeholder={t('password.currentPlaceholder')}
          />
          <Input
            variant="dark"
            label={t('password.new')}
            type="password"
            value={newPassword}
            onChange={(e) => setNewPassword(e.target.value)}
            placeholder={t('password.newPlaceholder')}
            helpText={t('frag.passwordMinHint', { n: MIN_PASSWORD_LENGTH })}
          />
          <Input
            variant="dark"
            label={t('password.confirm')}
            type="password"
            value={confirmPassword}
            onChange={(e) => setConfirmPassword(e.target.value)}
            placeholder={t('password.confirmPlaceholder')}
            error={confirmPassword && newPassword !== confirmPassword ? t('settingsErr.passwordMismatch') : undefined}
          />
        </div>
      </Modal>

      {/* Delete Provider Confirmation Dialog */}
      <ConfirmDialog
        isOpen={!!providerToDelete}
        onClose={() => setProviderToDelete(null)}
        onConfirm={() => providerToDelete && deleteMutation.mutate(providerToDelete)}
        title={t('credentials.remove')}
        message={t('app.confirmRemoveProviderCredsWeb', { provider: getProviderInfo(providerToDelete || '').name })}
        confirmLabel={t('app.remove')}
        variant="danger"
        isLoading={deleteMutation.isPending}
      />

      {/* Revoke Token Confirmation */}
      <ConfirmDialog
        isOpen={tokenToRevoke !== null}
        onClose={() => setTokenToRevoke(null)}
        onConfirm={() => tokenToRevoke && revokeTokenMutation.mutate(tokenToRevoke.id)}
        title={t('tokens.revoke')}
        message={t('app.confirmRevokeTokenWeb', { token: tokenToRevoke?.name ?? '' })}
        confirmLabel={t('app.revokeTokenTitle')}
        cancelLabel={t('common.cancel')}
        variant="danger"
        isLoading={revokeTokenMutation.isPending}
      />

      {/* Revoke Connected App Confirmation */}
      <ConfirmDialog
        isOpen={appToRevoke !== null}
        onClose={() => setAppToRevoke(null)}
        onConfirm={() => appToRevoke && revokeConnectedAppMutation.mutate(appToRevoke.id)}
        title={t('tokens.revokeAccess')}
        message={t('app.confirmRevokeAppAccess', { app: appToRevoke?.client_id ?? '' })}
        confirmLabel={t('app.revoke')}
        variant="danger"
        isLoading={revokeConnectedAppMutation.isPending}
      />

      {/* Disconnect Fitness Provider Confirmation */}
      <ConfirmDialog
        isOpen={providerToDisconnect !== null}
        onClose={() => setProviderToDisconnect(null)}
        onConfirm={() => providerToDisconnect && handleDisconnectProvider(providerToDisconnect)}
        title={t('providers.disconnectProvider')}
        message={t('app.confirmDisconnect', { provider: providerToDisconnect })}
        confirmLabel={t('app.disconnect')}
        variant="danger"
      />

      {/* Provider conflict confirmation (Strava vs Sciotte) */}
      <ConfirmDialog
        isOpen={providerConflict !== null}
        onClose={() => setProviderConflict(null)}
        onConfirm={async () => {
          if (!providerConflict) return;
          const disconnecting = providerConflict.disconnecting === 'Strava — Sciotte' ? 'sciotte' : 'strava';
          const connecting = providerConflict.connecting === 'Strava — Sciotte' ? 'sciotte' : 'strava';
          // Pre-open the OAuth popup synchronously here, before the disconnect
          // await consumes the click's user gesture. Mobile Safari otherwise
          // silently blocks the popup and the connect spinner runs to timeout.
          // Sciotte uses a credential modal, not an OAuth popup — skip the
          // preopen in that branch.
          const preopenedPopup = connecting === 'sciotte' ? null : window.open('about:blank', '_blank');
          await handleDisconnectProvider(disconnecting);
          setProviderConflict(null);
          if (connecting === 'sciotte') {
            setSciotteModalTarget('strava');
          } else {
            handleConnectProvider(connecting, preopenedPopup);
          }
        }}
        title={t('providers.switchProvider')}
        message={t('app.providerConflictWarning', { connecting: providerConflict?.connecting ?? '', disconnecting: providerConflict?.disconnecting ?? '' })}
        confirmLabel={t('app.switch')}
        variant="danger"
      />

      {/* Sciotte login modal */}
      <SciotteLoginModal
        isOpen={sciotteModalTarget !== null}
        onClose={() => setSciotteModalTarget(null)}
        onConnected={() => {
          refetchProviders();
          setSciotteModalTarget(null);
        }}
        target={sciotteModalTarget ?? 'strava'}
      />

      {/* Intervals.icu API-key link modal */}
      <IntervalsIcuLinkModal
        isOpen={intervalsModalOpen}
        onClose={() => setIntervalsModalOpen(false)}
        onConnected={() => {
          refetchProviders();
          setIntervalsModalOpen(false);
        }}
      />
    </div>
  );
}
