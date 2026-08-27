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
import { Card, Button, Badge, ConfirmDialog, Input, Modal, ModalActions, Select, useErrorToast } from './ui';
import { clsx } from 'clsx';
import A2AClientList from './A2AClientList';
import CreateA2AClient from './CreateA2AClient';
import CoachingPersonaTab from './CoachingPersonaTab';
import LlmSettingsTab from './LlmSettingsTab';
import MessagingSettingsTab from './MessagingSettingsTab';
import NotificationSettingsTab from './NotificationSettingsTab';
import PrivacySettingsTab from './PrivacySettingsTab';
import MemoryPanel from './memory/MemoryPanel';
import { buildFitnessProviderCards } from '../utils/fitnessProviderCards';
import { QUERY_KEYS } from '../constants/queryKeys';
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
  { id: 'whoop', name: 'WHOOP', color: 'bg-black' },
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

/** Format ISO 8601 reset time in user's local timezone */
function formatResetTime(isoString: string): string {
  try {
    const date = new Date(isoString);
    return new Intl.DateTimeFormat(undefined, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(date);
  } catch {
    return 'midnight UTC';
  }
}

type SettingsTab = 'profile' | 'connections' | 'tokens' | 'llm' | 'coaching' | 'messaging' | 'notifications' | 'memory' | 'privacy' | 'about' | 'account';

const SETTINGS_TABS: { id: SettingsTab; nameKey: string; icon: React.ReactNode }[] = [
  {
    id: 'profile',
    nameKey: 'settingsTabs.profile',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" />
      </svg>
    ),
  },
  // Data Providers ('connections') is intentionally absent here — it is a
  // top-level sidebar tab now, rendered via <UserSettings initialTab="connections"
  // hideTabNav /> so it is no longer buried under Profile/Settings.
  {
    id: 'tokens',
    nameKey: 'settingsTabs.tokens',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
      </svg>
    ),
  },
  {
    id: 'llm',
    nameKey: 'settingsTabs.ai',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
      </svg>
    ),
  },
  {
    id: 'coaching',
    nameKey: 'settingsTabs.coaching',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z" />
      </svg>
    ),
  },
  {
    id: 'messaging',
    nameKey: 'settingsTabs.messaging',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
      </svg>
    ),
  },
  {
    id: 'notifications',
    nameKey: 'settingsTabs.notifications',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9" />
      </svg>
    ),
  },
  {
    id: 'memory',
    nameKey: 'settingsTabs.memory',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
      </svg>
    ),
  },
  {
    id: 'privacy',
    nameKey: 'settingsTabs.privacy',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
      </svg>
    ),
  },
  {
    id: 'about',
    nameKey: 'settingsTabs.about',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
      </svg>
    ),
  },
  {
    id: 'account',
    nameKey: 'settingsTabs.account',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
      </svg>
    ),
  },
];

const ADMIN_HIDDEN_TABS: Set<SettingsTab> = new Set(['connections', 'about', 'messaging']);

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
      ? SETTINGS_TABS.filter(tab => !ADMIN_HIDDEN_TABS.has(tab.id))
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
    // otherwise show stale "Connect" here until a manual reconnect.
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
      setCredentialMessage({ type: 'error', text: error.message || 'Failed to save credentials' });
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
      setCredentialMessage({ type: 'error', text: error.message || 'Failed to remove credentials' });
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
      setMessage({ type: 'error', text: error.message || 'Failed to update profile' });
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
      setPasswordMessage({ type: 'error', text: error.message || 'Failed to change password' });
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
                setProviderMessage({ type: 'success', text: `${providerId} connected successfully!` });
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
    whoop: { color: '#000000', description: t('providerBlurb.whoop') },
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
    return PROVIDERS.find((p) => p.id === providerId) || { id: providerId, name: providerId, color: 'bg-surface-container-low0' };
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
                'snap-start flex items-center gap-2 px-4 py-3 text-sm font-medium whitespace-nowrap transition-all duration-200 border-b-2 min-h-[44px]',
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
      <div className="space-y-6">
        {/* Profile Tab */}
        {activeTab === 'profile' && (
          <>
            <Card variant="dark">
              <h2 className="text-lg font-semibold text-on-surface mb-4">{t('profile.title')}</h2>
              <div className="space-y-4">
                {/* Gradient ring avatar — stacks on mobile so the email
                    field (frequently long) doesn't get squeezed. */}
                <div className="flex flex-col sm:flex-row sm:items-center gap-4 pb-4 border-b ghost-border">
                  <div className="relative flex-shrink-0 self-start sm:self-auto">
                    <div className="w-20 h-20 sm:w-24 sm:h-24 rounded-full p-[3px] bg-gradient-to-br boreal-hero-gradient">
                      <div className="w-full h-full bg-surface-container-low rounded-full flex items-center justify-center">
                        <span className="text-3xl font-bold text-on-surface">
                          {(user?.display_name || user?.email)?.charAt(0).toUpperCase()}
                        </span>
                      </div>
                    </div>
                  </div>
                  <div className="min-w-0">
                    <p className="text-xl font-semibold text-on-surface break-words">{user?.display_name || 'No name set'}</p>
                    <p className="text-sm text-on-surface-variant break-all">{user?.email}</p>
                    <Badge variant={user?.user_status === 'active' ? 'success' : 'warning'} className="mt-1">
                      {user?.user_status?.charAt(0).toUpperCase()}{user?.user_status?.slice(1)}
                    </Badge>
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
                  <label className="block text-sm font-medium text-on-surface mb-2">{t('common.email')}</label>
                  <p className="text-on-surface-variant bg-surface-container-low px-4 py-3 rounded-md border ghost-border">{user?.email}</p>
                  <p className="text-xs text-outline mt-1">{t('profile.emailLocked')}</p>
                </div>

                {message && (
                  <div
                    className={`p-3 rounded-lg text-sm ${
                      message.type === 'success'
                        ? 'bg-activity/20 text-activity border border-activity/30'
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
                  className="shadow-ambient hover:shadow-ambient"
                >
                  {t('settings.saveChanges')}
                </Button>
              </div>
            </Card>

            {/* Appearance — the theme control's only reachable home once a
                user is signed in; the Login screen's toggle is gone by then. */}
            <Card variant="dark">
              <h2 className="text-lg font-semibold text-on-surface mb-4">{t('settings.appearance')}</h2>
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
                  className="flex items-center gap-2 px-4 py-2 rounded-lg text-on-surface bg-surface-container-low border ghost-border hover:bg-surface-container transition-colors min-h-[44px]"
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
              <div className="mt-6 pt-6 border-t ghost-border flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <p className="font-medium text-on-surface">{t('settings.language')}</p>
                  <p className="text-sm text-on-surface-variant">{t('settings.languageDescription')}</p>
                </div>
                <LanguageSwitcher serverLocale={user?.locale} />
              </div>
            </Card>

            {/* Quick Stats with gradient accent */}
            <div className="grid grid-cols-2 gap-4">
              <div className="stat-card-dark rounded-xl border ghost-border p-6">
                <div className="text-center">
                  <div className="text-3xl font-bold bg-gradient-to-r boreal-hero-gradient bg-clip-text text-transparent">
                    {statsLoading ? '...' : (stats?.connected_providers ?? 0)}
                  </div>
                  <div className="text-sm text-on-surface-variant mt-1">{t('providers.connectedTitle')}</div>
                </div>
              </div>
              <div className="stat-card-dark rounded-xl border ghost-border p-6">
                <div className="text-center">
                  <div className="text-3xl font-bold bg-gradient-to-r from-nutrition to-activity bg-clip-text text-transparent">
                    {statsLoading ? '...' : (stats?.days_active ?? 0)}
                  </div>
                  <div className="text-sm text-on-surface-variant mt-1">{t('profile.daysActive')}</div>
                </div>
              </div>
            </div>
          </>
        )}

        {/* Connections Tab */}
        {activeTab === 'connections' && (
          <>
            {/* Fitness Providers - Connection Status */}
            <Card variant="dark">
              <h2 className="text-lg font-semibold text-on-surface mb-1">{t('providers.fitnessTitle')}</h2>
              <p className="text-sm text-on-surface-variant mb-4">
                {t('providers.fitnessHint')}
              </p>

              {providerMessage && (
                <div
                  className={clsx(
                    'p-3 rounded-lg text-sm mb-4',
                    providerMessage.type === 'success'
                      ? 'bg-activity/20 text-activity border border-activity/30'
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
                <div className="text-center py-8 text-on-surface-variant">
                  <p>{t('providers.none')}</p>
                </div>
              ) : (
                <div className="space-y-3">
                  {fitnessProviders.map((provider) => {
                    const display = PROVIDER_DISPLAY[provider.provider] || {
                      color: '#607D8B',
                      description: t('providerBlurb.generic'),
                    };
                    const isConnecting = connectingProvider === provider.provider;

                    return (
                      <div
                        key={provider.provider}
                        className={clsx(
                          'p-4 rounded-xl border transition-all',
                          provider.connected
                            ? 'border-activity/30 bg-activity/10'
                            : 'ghost-border bg-surface-container-low'
                        )}
                      >
                        <div className="flex items-center gap-3">
                          <div
                            className="w-10 h-10 rounded-lg flex items-center justify-center flex-shrink-0"
                            style={{ backgroundColor: display.color }}
                          >
                            <span className="text-on-surface font-bold text-sm">
                              {provider.display_name.charAt(0)}
                            </span>
                          </div>
                          <div className="flex-1 min-w-0">
                            <div className="flex items-center gap-2">
                              <p className="font-medium text-on-surface">{provider.display_name}</p>
                              {provider.connected && (
                                provider.needs_reauth ? (
                                  <Badge variant="warning">{t('providers.reconnectNeeded')}</Badge>
                                ) : (
                                  <Badge variant="success">{t('settingsUi.connected')}</Badge>
                                )
                              )}
                            </div>
                            <p className="text-sm text-on-surface-variant truncate">{display.description}</p>
                            {provider.capabilities.length > 0 && (
                              <p className="text-xs text-outline mt-0.5">
                                {provider.capabilities.join(', ')}
                              </p>
                            )}
                          </div>
                          <div className="flex-shrink-0">
                            {/* A connected-but-dead session (needs_reauth) falls through to the
                                connect flow below, relabelled "Reconnect", so the user can revive
                                it rather than being stuck looking at a healthy-seeming row. */}
                            {provider.connected && !provider.needs_reauth ? (
                              (provider.requires_oauth || provider.provider.startsWith('sciotte') || provider.provider === 'intervals_icu') && (
                                <Button
                                  variant="secondary"
                                  size="sm"
                                  onClick={() => setProviderToDisconnect(provider.connectionProvider)}
                                  className="text-error hover:bg-error/20"
                                >
                                  {t('settingsUi.disconnect')}
                                </Button>
                              )
                            ) : provider.provider === 'intervals_icu' ? (
                              <Button
                                variant="gradient"
                                size="sm"
                                onClick={() => setIntervalsModalOpen(true)}
                              >
                                {provider.needs_reauth ? t('settingsUi.reconnect') : t('shell.intervalsConnectAction')}
                              </Button>
                            ) : provider.provider === 'sciotte' || provider.provider === 'sciotte_garmin' ? (
                              <Button
                                variant="gradient"
                                size="sm"
                                onClick={() => {
                                  // The `sciotte` card is the user-facing "Strava" card. OAuth is
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
                                variant="gradient"
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
              <div className="mt-4 p-3 bg-surface-container-low border ghost-border rounded-lg">
                <p className="text-xs text-on-surface-variant">
                  {t('providers.privacyNote')}
                </p>
              </div>
            </Card>

            {/* OAuth App Credentials (Advanced) */}
            <Card variant="dark">
            <div className="flex justify-between items-center mb-4">
              <div>
                <h2 className="text-lg font-semibold text-on-surface">{t('credentials.title')}</h2>
                <p className="text-sm text-on-surface-variant mt-1">
                  {t('credentials.useOwnHint')}
                </p>
              </div>
              {availableProviders.length > 0 && (
                <Button variant="secondary" size="sm" onClick={() => setShowAddCredentials(true)}>
                  {t('providers.add')}
                </Button>
              )}
            </div>

            {credentialMessage && (
              <div
                className={`p-3 rounded-lg text-sm mb-4 ${
                  credentialMessage.type === 'success'
                    ? 'bg-activity/20 text-activity border border-activity/30'
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
              <div className="text-center py-8 bg-surface-container-low rounded-xl border ghost-border">
                <svg
                  className="w-12 h-12 text-on-surface-variant mx-auto mb-3"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={1.5}
                    d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"
                  />
                </svg>
                <p className="text-on-surface font-medium">{t('credentials.empty')}</p>
                <p className="text-sm text-outline mt-1">
                  {t('credentials.addHint')}
                </p>
              </div>
            ) : (
              <div className="space-y-3">
                {oauthApps.map((app) => {
                  const provider = getProviderInfo(app.provider);
                  return (
                    <div key={app.provider} className="flex items-center justify-between p-4 bg-surface-container-low rounded-xl border ghost-border">
                      <div className="flex items-center gap-3">
                        <div className={`w-10 h-10 ${provider.color} rounded-lg flex items-center justify-center`}>
                          <span className="text-on-surface font-bold text-sm">{provider.name.charAt(0)}</span>
                        </div>
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
                    ? 'bg-activity/20 text-activity border border-activity/30'
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
          </Card>
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

            <Card variant="dark">
              <div className="flex justify-between items-center mb-4">
                <div>
                  <h2 className="text-lg font-semibold text-on-surface">{t('tokens.title')}</h2>
                  <p className="text-sm text-on-surface-variant mt-1">
                    {activeTokens.length} active tokens for AI client connections
                  </p>
                </div>
              </div>

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
                <div className="text-center py-8 text-on-surface-variant">
                  <svg className="w-12 h-12 text-on-surface-variant mx-auto mb-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
                  </svg>
                  <p className="text-lg mb-2 text-on-surface">{t('tokens.empty')}</p>
                  <p>{t('tokens.createHint')}</p>
                </div>
              ) : (
                <div className="space-y-4">
                  {tokens.map((token) => (
                    <div key={token.id} className="p-4 bg-surface-container-low border ghost-border rounded-lg">
                      <div className="flex items-start justify-between">
                        <div className="flex-1">
                          <div className="flex items-center gap-2">
                            <h3 className="text-lg font-medium text-on-surface">{token.name}</h3>
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
            </Card>

            {/* Connected Apps Section */}
            <Card variant="dark">
              <div className="flex justify-between items-center mb-4">
                <div>
                  <h2 className="text-lg font-semibold text-on-surface">{t('tokens.connectedApps')}</h2>
                  <p className="text-sm text-on-surface-variant mt-1">
                    {t('providers.thirdPartyHint')}
                  </p>
                </div>
              </div>
              {showCreateA2AClient ? (
                <CreateA2AClient
                  onSuccess={() => setShowCreateA2AClient(false)}
                  onCancel={() => setShowCreateA2AClient(false)}
                />
              ) : (
                <A2AClientList onCreateClient={() => setShowCreateA2AClient(true)} />
              )}
            </Card>
          </>
        )}

        {/* AI Settings Tab */}
        {activeTab === 'llm' && <LlmSettingsTab />}

        {activeTab === 'coaching' && <CoachingPersonaTab />}

        {activeTab === 'messaging' && <MessagingSettingsTab />}

        {activeTab === 'notifications' && <NotificationSettingsTab />}

        {activeTab === 'memory' && <MemoryPanel />}

        {activeTab === 'privacy' && <PrivacySettingsTab />}

        {/* About Tab */}
        {activeTab === 'about' && (
          <Card variant="dark">
            <h2 className="text-lg font-semibold text-on-surface mb-6">{t('about.title')}</h2>
            <div className="space-y-3">
              {/* Version */}
              <div className="flex items-center gap-4 p-4 bg-surface-container-low rounded-xl border ghost-border">
                <div className="w-10 h-10 rounded-xl bg-primary/15 flex items-center justify-center flex-shrink-0">
                  <svg className="w-5 h-5 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                  </svg>
                </div>
                <div className="flex-1">
                  <p className="text-sm text-on-surface-variant">{t('settingsUi.version')}</p>
                  <p className="text-on-surface font-medium">1.0.0</p>
                </div>
              </div>

              {/* Help Center */}
              <a
                href="https://dravr.ai/help"
                target="_blank"
                rel="noopener noreferrer"
                className="flex items-center gap-4 p-4 bg-surface-container-low rounded-xl border ghost-border hover:bg-surface-container transition-colors group"
              >
                <div className="w-10 h-10 rounded-xl bg-primary-container/15 flex items-center justify-center flex-shrink-0">
                  <svg className="w-5 h-5 text-primary-container" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M18.364 5.636l-3.536 3.536m0 5.656l3.536 3.536M9.172 9.172L5.636 5.636m3.536 9.192l-3.536 3.536M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-5 0a4 4 0 11-8 0 4 4 0 018 0z" />
                  </svg>
                </div>
                <div className="flex-1">
                  <p className="text-on-surface font-medium">{t('about.helpCenter')}</p>
                  <p className="text-sm text-on-surface-variant">{t('about.helpHint')}</p>
                </div>
                <svg className="w-5 h-5 text-outline group-hover:text-on-surface transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                </svg>
              </a>

              {/* Terms & Privacy */}
              <a
                href="https://dravr.ai/privacy"
                target="_blank"
                rel="noopener noreferrer"
                className="flex items-center gap-4 p-4 bg-surface-container-low rounded-xl border ghost-border hover:bg-surface-container transition-colors group"
              >
                <div className="w-10 h-10 rounded-xl bg-activity/15 flex items-center justify-center flex-shrink-0">
                  <svg className="w-5 h-5 text-activity" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                  </svg>
                </div>
                <div className="flex-1">
                  <p className="text-on-surface font-medium">{t('settingsUi.termsPrivacy')}</p>
                  <p className="text-sm text-on-surface-variant">{t('about.legalHint')}</p>
                </div>
                <svg className="w-5 h-5 text-outline group-hover:text-on-surface transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                </svg>
              </a>
            </div>
          </Card>
        )}

        {/* Account Tab */}
        {activeTab === 'account' && (
          <>
            <Card variant="dark">
              <h2 className="text-lg font-semibold text-on-surface mb-4">{t('profile.accountStatus')}</h2>
              <div className="space-y-3">
                <div className="flex justify-between items-center py-2 border-b ghost-border">
                  <span className="text-on-surface-variant">{t('settingsUi.status')}</span>
                  <span
                    className={`px-2 py-1 rounded-full text-xs font-medium ${
                      user?.user_status === 'active'
                        ? 'bg-success/20 text-success'
                        : 'bg-warning/20 text-warning'
                    }`}
                  >
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
            </Card>

            {/* Usage Quota Card */}
            {/* Usage quotas are user-facing only, not shown for admin */}
            {!isAdminUser && (
            <Card variant="dark">
              <div className="flex items-start gap-4 mb-5">
                <div className="w-10 h-10 rounded-xl bg-primary/15 flex items-center justify-center flex-shrink-0">
                  <svg className="w-5 h-5 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
                  </svg>
                </div>
                <div>
                  <h2 className="text-lg font-semibold text-on-surface">{t('settingsUi.usage')}</h2>
                  <p className="text-sm text-on-surface-variant">{t('account.quotaHint')}</p>
                </div>
              </div>

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
                    {t('frag.dailyLimitsResetAt')} {formatResetTime(usageData.daily.messages.resets_at)}
                  </p>

                  {/* Resource counts (user-facing only, not shown for admin) */}
                  {!isAdminUser && (
                  <div className="border-t ghost-border pt-4">
                    <div className="grid grid-cols-2 gap-4">
                      <div className="p-3 bg-surface-container-low rounded-lg">
                        <p className="text-xs text-outline mb-1">{t('settingsUi.coaches')}</p>
                        <p className="text-sm font-medium text-on-surface">
                          {usageData.resources.coaches} / {usageData.resources.max_coaches}
                        </p>
                      </div>
                      <div className="p-3 bg-surface-container-low rounded-lg">
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
            </Card>
            )}

            <Card variant="dark">
              <h2 className="text-lg font-semibold text-on-surface mb-4">{t('settingsUi.security')}</h2>
              <div className="space-y-4">
                <div className="p-4 bg-surface-container-low border ghost-border rounded-lg">
                  <h3 className="font-medium text-on-surface mb-2">{t('settingsUi.password')}</h3>
                  <p className="text-sm text-on-surface-variant mb-3">{t('password.changeHint')}</p>
                  <Button variant="outline" size="sm" onClick={() => setShowChangePassword(true)}>
                    {t('password.change')}
                  </Button>
                </div>
              </div>
            </Card>

            {/* Connected Apps — external OAuth clients (e.g. Claude Desktop) the
                user approved on the consent screen. Distinct from the A2A
                "Connected Apps" card in the API Tokens tab, which lists
                self-registered agent-to-agent clients. */}
            <Card variant="dark">
              <div className="mb-4">
                <h2 className="text-lg font-semibold text-on-surface">{t('tokens.connectedMcpApps')}</h2>
                <p className="text-sm text-on-surface-variant mt-1">
                  {t('tokens.connectedAppsHint')}
                </p>
              </div>

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
                <div className="space-y-3">
                  {connectedApps.map((app) => (
                    <div
                      key={app.id}
                      className="flex items-start justify-between gap-3 p-4 bg-surface-container-low border ghost-border rounded-lg"
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
                <div className="text-center py-8 text-on-surface-variant">
                  <p>{t('tokens.connectedAppsEmpty')}</p>
                </div>
              )}
            </Card>

            <Card variant="dark" className="border-error/30">
              <h2 className="text-lg font-semibold text-error mb-4">{t('account.dangerZone')}</h2>
              <div className="space-y-4">
                <div className="p-4 bg-error/10 border border-error/20 rounded-lg">
                  <h3 className="font-medium text-on-surface mb-2">{t('account.signOut')}</h3>
                  <p className="text-sm text-on-surface-variant mb-3">{t('account.signOutHint')}</p>
                  <Button variant="secondary" size="sm" onClick={logout}>
                    {t('account.signOut')}
                  </Button>
                </div>
              </div>
            </Card>
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
                  ? 'bg-activity/20 text-activity border border-activity/30'
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
        message={`Are you sure you want to remove the ${getProviderInfo(providerToDelete || '').name} credentials? You'll need to use the shared server credentials after this.`}
        confirmLabel="Remove"
        variant="danger"
        isLoading={deleteMutation.isPending}
      />

      {/* Revoke Token Confirmation */}
      <ConfirmDialog
        isOpen={tokenToRevoke !== null}
        onClose={() => setTokenToRevoke(null)}
        onConfirm={() => tokenToRevoke && revokeTokenMutation.mutate(tokenToRevoke.id)}
        title={t('tokens.revoke')}
        message={`Are you sure you want to revoke "${tokenToRevoke?.name}"? Any AI clients using this token will lose access immediately.`}
        confirmLabel="Revoke Token"
        cancelLabel="Cancel"
        variant="danger"
        isLoading={revokeTokenMutation.isPending}
      />

      {/* Revoke Connected App Confirmation */}
      <ConfirmDialog
        isOpen={appToRevoke !== null}
        onClose={() => setAppToRevoke(null)}
        onConfirm={() => appToRevoke && revokeConnectedAppMutation.mutate(appToRevoke.id)}
        title={t('tokens.revokeAccess')}
        message={`Revoke access for "${appToRevoke?.client_id}"? It will need to be re-authorized on its next connection.`}
        confirmLabel="Revoke"
        variant="danger"
        isLoading={revokeConnectedAppMutation.isPending}
      />

      {/* Disconnect Fitness Provider Confirmation */}
      <ConfirmDialog
        isOpen={providerToDisconnect !== null}
        onClose={() => setProviderToDisconnect(null)}
        onConfirm={() => providerToDisconnect && handleDisconnectProvider(providerToDisconnect)}
        title={t('providers.disconnectProvider')}
        message={`Are you sure you want to disconnect ${providerToDisconnect}? You will need to reconnect to sync new data.`}
        confirmLabel="Disconnect"
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
        message={`Connecting ${providerConflict?.connecting} will disconnect ${providerConflict?.disconnecting}. Both providers access Strava data — only one can be active at a time. Continue?`}
        confirmLabel="Switch"
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
