// ABOUTME: First-run onboarding screen that forces a provider connection before the user reaches chat
// ABOUTME: Mirrors the web OnboardingConnectProvider flow: Sciotte modal for sciotte providers, BYO modal for Whoop, awaiting-consent overlay

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useQueryClient } from '@tanstack/react-query';
import * as WebBrowser from 'expo-web-browser';
import * as Linking from 'expo-linking';
import { PRIMARY_PALETTE, PROVIDER_COLORS, useThemeColors } from '../../constants/theme';
import { Card, Button } from '../../components/ui';
import { SciotteLoginModal } from '../../components/SciotteLoginModal';
import { IntervalsIcuLinkModal } from '../../components/IntervalsIcuLinkModal';
import { OAuthAppSetupModal } from '../../components/OAuthAppSetupModal';
import { oauthApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { getOAuthCallbackUrl } from '../../utils/oauth';
import type { ExtendedProviderStatus } from '../../types';
import { useProviderSkipped } from '../../hooks/useProviderSkipped';
import { ConnectPreview } from '../../components/ConnectPreview';
import { useTranslation } from '@pierre/i18n';

/**
 * Backed by the same source of truth (`provider_connections`) as the
 * backend's `NoProviderConnected` 403 gate on chat/coach/messaging — so this
 * screen cannot drift from server-side enforcement.
 *
 * No back button: the user reaches this because RootLayoutNav saw
 * `needs_provider_connection: true`, and stepping backwards would land them on a
 * step they already finished.
 *
 * There IS a skip, matching web. This screen used to have none, which meant the
 * same account met a soft wall on a laptop and a hard one on a phone — the
 * harder wall being the surface people actually onboard on. The skip is
 * session-only (see `useProviderSkipped`), so the nudge returns next launch, and
 * the backend `NoProviderConnected` 403 still refuses any turn that would need
 * provider data. Skipping defers the ask; it does not buy access.
 */
export function OnboardingConnectScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const queryClient = useQueryClient();
  const { isAuthenticated, user, logout } = useAuth();
  const { skip } = useProviderSkipped(user?.id);
  const [providers, setProviders] = useState<ExtendedProviderStatus[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);
  const [connectError, setConnectError] = useState<string | null>(null);
  // Sciotte (credential login) target for the modal — null when closed.
  const [sciotteTarget, setSciotteTarget] = useState<'strava' | 'garmin' | null>(null);
  const [intervalsModalVisible, setIntervalsModalVisible] = useState(false);
  // Whoop is BYO-OAuth-app: users register their own developer app at
  // developer.whoop.com and paste client_id/secret before the OAuth dance can
  // run. Open the setup modal in-place so first-touch users never need to
  // leave the onboarding gate.
  const [showWhoopSetup, setShowWhoopSetup] = useState(false);
  // Tracks an in-flight OAuth in ASWebAuthenticationSession. Shows an
  // "awaiting consent" overlay with a Cancel button so the user is never
  // stranded if they background the app mid-flow.
  const [awaitingOAuthFor, setAwaitingOAuthFor] = useState<string | null>(null);
  // Bridges the gap between a successful connect and the RootLayoutNav route
  // flip: renders the "Provider connected — preparing your dashboard…"
  // spinner instead of the static cards while onboarding-status refetches.
  const [justConnected, setJustConnected] = useState<string | null>(null);

  const loadStatus = useCallback(async () => {
    try {
      setIsLoading(true);
      const response = await oauthApi.getProvidersStatus();
      setProviders(response.providers || []);
    } catch (err) {
      console.error('Failed to load provider list on onboarding:', err);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (isAuthenticated) {
      loadStatus();
    }
  }, [isAuthenticated, loadStatus]);

  // Safety net: if RootLayoutNav never flips (onboarding-status refetch races,
  // OAuth callback never lands, etc.) auto-revert to the cards so the user
  // isn't pinned on the spinner forever. 30s is plenty for a successful
  // refetch; anything longer is a hung state we should escape.
  useEffect(() => {
    if (!justConnected) return undefined;
    const timer = setTimeout(() => {
      setJustConnected(null);
      setConnectError(
        'Couldn’t confirm the connection. If you completed the connect flow, pull to refresh; otherwise try again.',
      );
    }, 30_000);
    return () => clearTimeout(timer);
  }, [justConnected]);

  const finalizeConnection = useCallback(
    async (providerName: string) => {
      // Invalidate the onboarding-status query so RootLayoutNav re-evaluates
      // and routes the user into the chat stack.
      await queryClient.invalidateQueries({ queryKey: ['user-onboarding-status'] });
      await loadStatus();
      setJustConnected(providerName);
    },
    [queryClient, loadStatus],
  );

  // Standard OAuth flow used by providers with a built-in client_id/secret on
  // the platform (currently only WHOOP once the user has registered a BYO
  // app). Sciotte providers handle their own credential flow via the modal.
  const launchOAuth = useCallback(
    async (providerId: string, providerName: string) => {
      try {
        setConnectingProvider(providerId);
        setConnectError(null);
        setAwaitingOAuthFor(providerId);
        const returnUrl = getOAuthCallbackUrl();
        const oauthResponse = await oauthApi.initMobileOAuth(providerId, returnUrl);
        const result = await WebBrowser.openAuthSessionAsync(
          oauthResponse.authorization_url,
          returnUrl,
        );
        setAwaitingOAuthFor(null);

        if (result.type === 'success' && result.url) {
          if (!result.url.startsWith(returnUrl)) {
            Alert.alert(t('app.connectionFailed'), 'Unexpected OAuth callback URL');
            return;
          }
          const parsed = Linking.parse(result.url);
          const success = parsed.queryParams?.success === 'true';
          const error = parsed.queryParams?.error as string | undefined;
          if (success) {
            await finalizeConnection(providerName);
          } else if (error) {
            // Strava OAuth came back with an error (e.g. the shared-app athlete
            // cap was actually exceeded in a seat-count race, or the provider
            // rejected the grant). Don't strand the user — fall back to the
            // Sciotte credential login, which serves the same Strava data.
            if (providerId === 'strava') {
              setSciotteTarget('strava');
            } else {
              setConnectError(`Failed to connect ${providerName}: ${error}`);
            }
          } else {
            // No explicit success/error — refetch to see if the row landed.
            await finalizeConnection(providerName);
          }
        }
        // type === 'cancel' / 'dismiss' — user backed out of the auth sheet;
        // that's a deliberate choice, so we do NOT push the Sciotte fallback.
      } catch (err) {
        setAwaitingOAuthFor(null);
        const message = err instanceof Error ? err.message : t('app.failedToConnect');
        console.error('Onboarding OAuth flow failed:', err);
        if (providerId === 'whoop') {
          // No BYO app registered yet — open the in-place setup modal so the
          // first-run user never needs to navigate to Settings.
          setShowWhoopSetup(true);
          return;
        }
        // Couldn't even start the Strava OAuth flow (init/network error). Fall
        // back to the Sciotte credential login rather than leaving the user
        // stuck on an error toast.
        if (providerId === 'strava') {
          setSciotteTarget('strava');
          return;
        }
        setConnectError(message);
      } finally {
        setConnectingProvider(null);
      }
    },
    [finalizeConnection, t],
  );

  const handleConnect = (provider: ExtendedProviderStatus) => {
    setConnectError(null);
    // The Sciotte card is the user-facing t('app.brandStrava') card. While shared-app OAuth
    // seats remain the server recommends `oauth`, so connect via the official
    // Strava OAuth flow. Once the athlete cap is reached the server recommends
    // `mirror`, and we silently fall back to the Sciotte credential login.
    if (provider.provider === 'sciotte') {
      if (provider.recommended_backend === 'oauth') {
        void launchOAuth('strava', provider.display_name);
      } else {
        setSciotteTarget('strava');
      }
      return;
    }
    // Other Sciotte backends (Garmin) always use credential-based login.
    if (provider.provider.startsWith('sciotte')) {
      setSciotteTarget('garmin');
      return;
    }
    if (provider.provider === 'intervals_icu') {
      setIntervalsModalVisible(true);
      return;
    }
    if (provider.provider === 'whoop') {
      // Skip the speculative OAuth init for Whoop — open the setup modal
      // directly. Modal pre-populates from any existing app, so returning
      // users only need to re-enter the secret (which we never persist).
      setShowWhoopSetup(true);
      return;
    }
    void launchOAuth(provider.provider, provider.display_name);
  };

  // After the 2026-Q2 provider cleanup the API surfaces only three: `sciotte`
  // (Strava-branded), `sciotte_garmin` (Garmin-branded), and `whoop`. Filter
  // out the bare `strava` row — official OAuth is reached exclusively through
  // the Sciotte modal's t('app.useOwnStravaApp') button, so a separate
  // strava card would just duplicate the entry. Mirror its `connected` state
  // onto the Sciotte card so the badge appears in the right place.
  const visibleProviders = (() => {
    const stravaConnected = providers.find((p) => p.provider === 'strava' && p.connected);
    return providers
      .filter((p) => p.provider !== 'strava')
      .filter((p) => p.requires_oauth || p.provider.startsWith('sciotte') || p.provider === 'intervals_icu')
      .map((p) =>
        p.provider === 'sciotte' && stravaConnected && !p.connected
          ? { ...p, connected: true }
          : p,
      );
  })();

  const renderProvider = (provider: ExtendedProviderStatus) => {
    // Provider display config (colors, icons, descriptions). Unknown ids fall
    // back to a neutral slate tile so the screen never crashes on an
    // unexpected payload.
    const config: Record<string, { color: string; icon: string; description: string }> = {
      sciotte: { color: PROVIDER_COLORS.strava, icon: 'S', description: t('app.provRunCycleSwim') },
      sciotte_garmin: { color: PROVIDER_COLORS.garmin, icon: 'G', description: t('app.provActivitiesHealth') },
      whoop: { color: PROVIDER_COLORS.whoop, icon: 'W', description: t('app.provRecoveryStrainSleep') },
      intervals_icu: { color: '#1273DE', icon: 'I', description: t('app.provEnduranceWellness') },
    };
    const c = config[provider.provider] ?? { color: '#607D8B', icon: '?', description: t('app.provFitnessData') };
    const isConnecting = connectingProvider === provider.provider;
    const isConnected = provider.connected;

    return (
      <Card key={provider.provider} className="mb-3">
        <View className="flex-row items-center">
          <View
            className="w-11 h-11 rounded-xl items-center justify-center mr-3"
            style={{ backgroundColor: c.color }}
          >
            <Text className="text-xl font-bold text-on-surface">{c.icon}</Text>
          </View>
          <View className="flex-1 mr-3">
            <Text className="text-base font-semibold text-text-primary">{provider.display_name}</Text>
            <Text className="text-xs text-text-secondary mt-0.5" numberOfLines={1}>{c.description}</Text>
          </View>
          {isConnected ? (
            <View className="bg-success/15 px-3 py-1.5 rounded-full">
              <Text className="text-xs text-success font-semibold">{t('app.connected')}</Text>
            </View>
          ) : (
            <TouchableOpacity
              className="px-5 py-2 rounded-full"
              style={{ backgroundColor: c.color }}
              onPress={() => handleConnect(provider)}
              disabled={isConnecting}
              activeOpacity={0.7}
              accessibilityLabel={`Connect ${provider.display_name}`}
            >
              {isConnecting ? (
                <ActivityIndicator size="small" color="#FFFFFF" />
              ) : (
                <Text className="text-sm font-semibold text-on-surface">{t('app.connect')}</Text>
              )}
            </TouchableOpacity>
          )}
        </View>
      </Card>
    );
  };

  // Post-connect spinner: full-screen so the static t('app.connected') badge doesn't
  // flash before RootLayoutNav flips. Matches the web onboarding UX.
  if (justConnected) {
    return (
      <SafeAreaView className="flex-1 bg-background-primary items-center justify-center px-8">
        <ActivityIndicator size="large" color={PRIMARY_PALETTE[500]} />
        <Text className="mt-4 text-base font-medium text-text-primary text-center">
          {justConnected} connected — preparing your dashboard…
        </Text>
      </SafeAreaView>
    );
  }

  // Awaiting OAuth consent — covers the gap between
  // openAuthSessionAsync starting and the user returning from the in-app
  // browser. Cancel button drops back to the cards so they're never trapped.
  if (awaitingOAuthFor) {
    const friendlyName = awaitingOAuthFor.charAt(0).toUpperCase() + awaitingOAuthFor.slice(1);
    return (
      <SafeAreaView className="flex-1 bg-background-primary items-center justify-center px-8">
        <ActivityIndicator size="large" color={PRIMARY_PALETTE[500]} />
        <Text className="mt-4 text-base font-semibold text-text-primary text-center">
          {t('app.awaiting')} {friendlyName} consent…
        </Text>
        <Text className="mt-2 text-sm text-text-tertiary text-center">
          {t('app.finishAuthInBrowser', { provider: friendlyName })}
        </Text>
        <View className="mt-6">
          <Button
            title={t('app.cancelTryDifferentProvider')}
            variant="secondary"
            onPress={() => setAwaitingOAuthFor(null)}
          />
        </View>
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="onboarding-screen">
      <ScrollView contentContainerStyle={{ padding: 20, paddingBottom: 40 }}>
        <View className="mb-6 mt-4">
          <Text
            className="text-3xl font-bold text-text-primary mb-3"
            accessibilityRole="header"
          >
            {user?.display_name ? `Welcome, ${user.display_name}` : t('app.welcomeToDravr')}
          </Text>
          <Text className="text-base text-text-secondary leading-6">
            {t('app.obConnectBlurb')}
          </Text>
        </View>

        {isLoading ? (
          <View className="py-12 items-center">
            <ActivityIndicator size="large" color={colors.tokens.primary} />
          </View>
        ) : (
          visibleProviders.map(renderProvider)
        )}

        {connectError && (
          <View
            accessibilityLiveRegion="polite"
            className="mt-4 rounded-lg border border-error/40 bg-error/10 px-4 py-3"
          >
            <Text className="text-sm text-error">{connectError}</Text>
          </View>
        )}

        <Text className="text-xs text-text-tertiary text-center mt-6">
          {t('app.credsEncrypted')}
        </Text>

        <ConnectPreview />

        <View className="mt-6 items-center">
          <TouchableOpacity onPress={skip} accessibilityRole="button">
            <Text className="text-sm font-medium text-text-secondary underline">
              {t('app.continueWithoutConnecting')}
            </Text>
          </TouchableOpacity>
          <Text className="text-xs text-text-tertiary text-center mt-1">
            {t('app.connectAnytime')}
          </Text>
        </View>

        <View className="mt-8">
          <Button
            title={t('common.logout')}
            variant="secondary"
            onPress={() => void logout()}
            fullWidth
          />
        </View>
      </ScrollView>

      <SciotteLoginModal
        visible={sciotteTarget !== null}
        onClose={() => setSciotteTarget(null)}
        onConnected={() => {
          const target = sciotteTarget ?? 'strava';
          const friendly = target === 'garmin' ? t('app.brandGarmin') : t('app.brandStrava');
          setSciotteTarget(null);
          void finalizeConnection(friendly);
        }}
        target={sciotteTarget ?? 'strava'}
      />

      <IntervalsIcuLinkModal
        visible={intervalsModalVisible}
        onClose={() => setIntervalsModalVisible(false)}
        onConnected={() => {
          setIntervalsModalVisible(false);
          void finalizeConnection('Intervals.icu');
        }}
      />

      <OAuthAppSetupModal
        visible={showWhoopSetup}
        onClose={() => setShowWhoopSetup(false)}
        onSaved={() => {
          setShowWhoopSetup(false);
          // BYO credentials are persisted; kick off the standard OAuth dance.
          // launchOAuth handles success → finalizeConnection.
          void launchOAuth('whoop', 'WHOOP');
        }}
        provider="whoop"
        displayName="WHOOP"
        devPortalUrl="https://developer.whoop.com/"
      />
    </SafeAreaView>
  );
}
