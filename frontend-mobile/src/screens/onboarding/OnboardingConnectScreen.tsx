// ABOUTME: First-run onboarding screen that forces a provider connection before the user reaches chat — and, in seat mode, the coach-seat step
// ABOUTME: Mirrors the web OnboardingConnectProvider / OnboardingCoachSeat: each card's connect plan, the history sign-in after a grant, BYO for Whoop, awaiting-consent overlay

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState, useEffect, useCallback, useRef } from 'react';
import {
  View,
  Text,
  ScrollView,
  StyleSheet,
  Pressable,
  ActivityIndicator,
  Alert,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Feather } from '@expo/vector-icons';
import { useQueryClient } from '@tanstack/react-query';
import * as WebBrowser from 'expo-web-browser';
import * as Linking from 'expo-linking';
import { useThemeColors } from '../../constants/theme';
import { Button } from '../../components/ui';
import { OnboardingProgressBar } from '../../components/ui/OnboardingProgressBar';
import { ProviderGlyph } from '../../components/ProviderGlyph';
import { ImportOwnDataRow } from '../../components/ImportOwnDataRow';
import { SciotteLoginModal } from '../../components/SciotteLoginModal';
import { IntervalsIcuLinkModal } from '../../components/IntervalsIcuLinkModal';
import { OAuthAppSetupModal } from '../../components/OAuthAppSetupModal';
import { ProviderNoticeSheet } from '../../components/ProviderNotice';
import { oauthApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { getOAuthCallbackUrl } from '../../utils/oauth';
import { promptImportTombstoneClear } from '../../utils/importTombstone';
import type { ExtendedProviderStatus } from '../../types';
import { useProviderSkipped } from '../../hooks/useProviderSkipped';
import { useConnectProviderDone } from '../../hooks/useConnectProviderDone';
import { useOnboardingProgress } from '../../hooks/useOnboardingProgress';
import { useOnboardingStatus } from '../../hooks/useOnboardingStatus';
import { COACH_SEAT_DONE_PREFIX, useOnboardingFlag } from '../../hooks/useOnboardingFlag';
import { CoachSeatPanel } from '../../components/CoachSeatPanel';
import { useSignInModal } from '../../hooks/useSignInModal';
import { importRouteFor, refusedConnectLine, type RefusedConnect } from '../../utils/connectFlow';
import { useTranslation } from '@pierre/i18n';
import type { SciotteTarget } from '@pierre/shared-types';
import {
  COACH_PLATFORM_PROVIDERS,
  HISTORY_ACTION_KEYS,
  OAUTH_NO_SEAT,
  ONBOARDING_LEAD_PROVIDERS,
  cardForSlug,
  cardStatus,
  connectPlan,
  credentialFallbackOnRefusal,
  noticeRequired,
  refusalReloadsCards,
  refusalReturnsToCard,
  sciotteTargetForBackend,
} from '@pierre/shared-constants';
import { useFocusEffect, useRouter } from 'expo-router';
import {
  cardAwaitingHistory,
  cardExportSource,
  describeApiError,
  importTombstoneFromReturn,
  refusalReason,
} from '@pierre/ui-logic';
import { ownAppDevPortal } from '@pierre/domain-utils';

/**
 * Backed by the same source of truth (`provider_connections`) as the
 * backend's `NoProviderConnected` 403 gate on chat/coach/messaging — so this
 * screen cannot drift from server-side enforcement.
 *
 * No back button: the user reaches this because RootLayoutNav saw
 * `needs_provider_connection: true`, and stepping backwards would land them on a
 * step they already finished.
 *
 * Tes données, the athlete's second screen (carnet#1530): « Connecte ton
 * entraînement », Garmin, Strava and Intervals.icu, the rest and the file
 * import behind « Plus de services ». « Continuer sans connecter » is a
 * visible secondary, matching web, and saved on the server (see
 * `useProviderSkipped`), so the screen never comes back at sign-in; the
 * backend `NoProviderConnected` 403 still refuses any turn that would need
 * provider data. Skipping defers the ask; it does not buy access. Sign out
 * sits quiet at the bottom.
 *
 * In `seat` mode it is the coach-seat step (web `OnboardingCoachSeat`): the
 * two coaching platforms as plain rows, in seat order, through the same
 * connect flows, with the link to ask the Dravr team and the seat as the
 * server has it below them (`CoachSeatPanel`). No activity-data line, import
 * or preview: these are accounts that decide coach access, not data to bring
 * in. A coach who already holds the seat when the step comes up is stepped
 * past without a screen.
 */
export function OnboardingConnectScreen({ mode = 'connect' }: { mode?: 'connect' | 'seat' } = {}) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const queryClient = useQueryClient();
  const { isAuthenticated, user, logout } = useAuth();
  const { skip } = useProviderSkipped(user?.id);
  const { complete: continuePastConnect } = useConnectProviderDone(user?.id);
  const seat = mode === 'seat';
  const progress = useOnboardingProgress(seat ? 'coach_seat' : 'connect_provider');
  const { mark: markSeatDone } = useOnboardingFlag(COACH_SEAT_DONE_PREFIX, user?.id);
  // Held before the step came up: nothing to get. Read once, so a seat
  // granted on this screen is shown rather than skipped.
  const heldOnArrival = useRef(seat && user?.manages_roster === true);
  useEffect(() => {
    if (heldOnArrival.current) void markSeatDone();
  }, [markSeatDone]);
  const router = useRouter();
  const { data: onboardingStatus } = useOnboardingStatus(isAuthenticated);
  const [providers, setProviders] = useState<ExtendedProviderStatus[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);
  const [connectError, setConnectError] = useState<string | null>(null);
  const [moreOpen, setMoreOpen] = useState(false);
  // The provider sign-in window: the account, the notice its row owes, and
  // whether it copies the history of a row already connected.
  const signIn = useSignInModal();
  const { open: openSignIn, openFor: openSignInFor } = signIn;
  // The row whose connect came back refused, and why when the server named
  // it: it says so, and a tap on that line starts its connect again.
  const [refusedCard, setRefusedCard] = useState<RefusedConnect | null>(null);
  // The Intervals.icu sheet for the row that opened it, and whether it leads
  // with the authorization (`intervalsConnectPath` answered `'choose'`) or is
  // the key form alone.
  const [intervalsSheet, setIntervalsSheet] = useState<{
    displayName: string;
    offersAuthorize: boolean;
  } | null>(null);
  // A provider no app of the server's can authorize for this athlete
  // (`own_app_required`) asks for an app of their own first: the setup sheet
  // opens in place, so first-touch users never need to leave the onboarding
  // gate, and saving it starts the OAuth flow with the notice acceptance that
  // led here.
  const [ownAppSetup, setOwnAppSetup] = useState<{
    provider: ExtendedProviderStatus;
    devPortalUrl: string;
    tosConsent: boolean;
  } | null>(null);
  // The OAuth provider whose notice is on screen before its flow starts
  // (WHOOP, until the account accepts its owner authorization).
  const [noticeFor, setNoticeFor] = useState<ExtendedProviderStatus | null>(null);
  // Tracks an in-flight OAuth in ASWebAuthenticationSession, by the name the
  // provider's card shows. Shows an "awaiting consent" overlay with a Cancel
  // button so the user is never stranded if they background the app mid-flow.
  const [awaitingOAuthFor, setAwaitingOAuthFor] = useState<string | null>(null);

  // Returns the providers it read (none when the read failed), so a connect
  // that just came back can judge the row it landed on.
  const loadStatus = useCallback(async (): Promise<ExtendedProviderStatus[]> => {
    try {
      setIsLoading(true);
      const response = await oauthApi.getProvidersStatus();
      const served = response.providers || [];
      setProviders(served);
      return served;
    } catch (err) {
      console.error('Failed to load provider list on onboarding:', err);
      return [];
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (isAuthenticated) {
      loadStatus();
    }
  }, [isAuthenticated, loadStatus]);

  // The seat panel's re-read while a connection settles the seat: the cards
  // again, for the account role, without the list's loading state.
  const reloadProviders = useCallback(() => {
    oauthApi
      .getProvidersStatus()
      .then((response) => setProviders(response.providers || []))
      .catch(() => {});
  }, []);

  // Back from the import of an export, the step may be done without a
  // connection (an import started): the onboarding status is read again.
  useFocusEffect(
    useCallback(() => {
      void queryClient.invalidateQueries({ queryKey: ['user-onboarding-status'] });
    }, [queryClient]),
  );

  // A connection landed. The screen stays up with the row marked connected so
  // a second service is one tap away; Continue moves on. The onboarding-status
  // refetch is what lets RootLayoutNav see the first connection.
  const finalizeConnection = useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ['user-onboarding-status'] });
    return loadStatus();
  }, [queryClient, loadStatus]);

  /**
   * Tell the row `cardKey` its connect did not go through. When the
   * refusal says the row's plan changed under the athlete (the seat it was
   * shown went), the rows are read again first, and a row that now signs in
   * opens its sign-in at once.
   */
  const showRefusal = useCallback(
    async (
      cardKey: string,
      { reload }: { reload: boolean },
    ) => {
      if (reload) {
        const served = await loadStatus();
        const card = served.find((p) => p.provider === cardKey);
        const primary = card ? connectPlan(card).primary : null;
        if (card && primary?.kind === 'sign_in') {
          openSignIn(primary.target, card);
          return;
        }
      }
      setRefusedCard({ card: cardKey });
    },
    [loadStatus, openSignIn],
  );
  // Continue opens once a row is connected, or once the server counts the
  // step done without one (an import of the athlete's own export started).
  const anyConnected = providers.some((p) => p.connected) || onboardingStatus?.needs_provider_connection === false;

  // Standard OAuth flow used by providers with a built-in client_id/secret on
  // the platform (currently only WHOOP once the user has registered a BYO
  // app). Sciotte providers handle their own credential flow via the modal.
  const launchOAuth = useCallback(
    async (providerId: string, providerName: string, tosConsent = false) => {
      // The row the authorization belongs to: `garmin` and `strava` land on
      // the card of their sign-in backend, whose row shows the spinner.
      const cardKey = cardForSlug(providerId, providers)?.provider ?? providerId;
      try {
        setConnectingProvider(cardKey);
        setConnectError(null);
        setAwaitingOAuthFor(providerName);
        const returnUrl = getOAuthCallbackUrl();
        const oauthResponse = await oauthApi.initMobileOAuth(providerId, returnUrl, { tosConsent });
        const result = await WebBrowser.openAuthSessionAsync(
          oauthResponse.authorization_url,
          returnUrl,
        );
        setAwaitingOAuthFor(null);

        if (result.type === 'success' && result.url) {
          if (!result.url.startsWith(returnUrl)) {
            Alert.alert(t('app.connectionFailed'), t('app.unexpectedOauthCallback'));
            return;
          }
          const parsed = Linking.parse(result.url);
          // A provider account a deleted Dravr account held: offered to
          // start fresh, never sent to the credential login, which would be
          // refused the same way.
          const tombstone = importTombstoneFromReturn((name) => parsed.queryParams?.[name]);
          if (tombstone) {
            promptImportTombstoneClear({
              refusal: tombstone,
              providerName,
              language,
              t,
              onCleared: () => void launchOAuthRef.current(providerId, providerName, tosConsent),
            });
            return;
          }
          const success = parsed.queryParams?.success === 'true';
          const error = parsed.queryParams?.error as string | undefined;
          if (success) {
            // A grant whose history is still needed opens the sign-in that
            // copies it, at once (carnet#1472).
            const served = await finalizeConnection();
            const owed = cardAwaitingHistory(served, providerId);
            if (owed) openSignIn(owed.target, owed.card, true);
          } else if (error) {
            // A refused Strava grant falls back to the Strava sign-in, which
            // serves the same data. A Garmin grant refused because the seats
            // ran out reads the rows again and follows the plan they now give
            // (the sign-in while the seats are out); any other refused Garmin
            // grant says so on its row, whose tap tries the authorization again.
            const fallback = credentialFallbackOnRefusal(providerId);
            if (fallback) {
              openSignInFor(fallback, providers);
            } else if (refusalReturnsToCard(providerId)) {
              await showRefusal(cardKey, { reload: error === OAUTH_NO_SEAT });
            } else {
              setConnectError(t('app.failedConnectProviderReason', { provider: providerName, reason: error }));
            }
          } else {
            // No explicit success/error — refetch to see if the row landed.
            await finalizeConnection();
          }
        }
        // type === 'cancel' / 'dismiss' — user backed out of the auth sheet;
        // that's a deliberate choice, so we do NOT push the Sciotte fallback.
      } catch (err) {
        setAwaitingOAuthFor(null);
        const message = describeApiError(err, { t, fallbackKey: 'app.failedToConnect' });
        console.error('Onboarding OAuth flow failed:', err);
        // Couldn't even start the Strava OAuth flow (init/network error):
        // fall back to the Strava sign-in rather than leaving the user stuck
        // on an error toast. Garmin says so on its row; a start refused
        // because its seats changed reads the rows again, and a row that now
        // signs in opens it.
        const fallback = credentialFallbackOnRefusal(providerId);
        if (fallback) {
          openSignInFor(fallback, providers);
          return;
        }
        if (refusalReturnsToCard(providerId)) {
          await showRefusal(cardKey, { reload: refusalReloadsCards(refusalReason(err)) });
          return;
        }
        setConnectError(message);
      } finally {
        setConnectingProvider(null);
      }
    },
    [finalizeConnection, language, openSignIn, openSignInFor, providers, showRefusal, t],
  );
  // The start-fresh action runs the connect again from inside the callback
  // that defined it, so it reads the current one through a ref.
  const launchOAuthRef = useRef(launchOAuth);
  useEffect(() => {
    launchOAuthRef.current = launchOAuth;
  }, [launchOAuth]);

  const handleConnect = (provider: ExtendedProviderStatus) => {
    setConnectError(null);
    setRefusedCard(null);
    // An OAuth provider whose notice the account has not accepted (WHOOP's
    // owner authorization) states it first; its Continue resumes below.
    if (noticeRequired(provider.provider, provider.consent_required) && !sciotteTargetForBackend(provider.provider)) {
      setNoticeFor(provider);
      return;
    }
    continueConnect(provider, false);
  };

  // A sign-in that could not complete hands over to its provider's OAuth
  // (carnet#1477): the window closes and the authorization starts instead.
  const fallBackToOAuth = (provider: SciotteTarget) => () => {
    const card = cardForSlug(provider, providers);
    signIn.close();
    void launchOAuth(provider, card?.display_name ?? provider);
  };

  /** The connect `handleConnect` resumes once any OAuth notice is accepted. */
  const continueConnect = (provider: ExtendedProviderStatus, tosConsent: boolean) => {
    // The card's `connectPlan` decides: the Strava and Garmin cards (named for
    // their sign-in backends) go through the provider's authorization while
    // the server recommends it and sign in otherwise; COROS and TrainingPeaks
    // sign in, after their notice when they have one.
    const primary = connectPlan(provider).primary;
    switch (primary.kind) {
      case 'sign_in':
        openSignIn(primary.target, provider, false, true);
        return;
      case 'intervals':
        // Intervals.icu leads with the athlete's authorization while the
        // server offers it, and keeps the coach's API-key form beside it; with
        // no client to authorize under, the key form is the only way in.
        setIntervalsSheet({ displayName: provider.display_name, offersAuthorize: primary.path === 'choose' });
        return;
      case 'oauth': {
        const devPortalUrl = ownAppDevPortal(provider.provider);
        if (provider.own_app_required && devPortalUrl) {
          setOwnAppSetup({ provider, devPortalUrl, tosConsent });
          return;
        }
        void launchOAuth(primary.provider, provider.display_name, tosConsent);
        return;
      }
      case 'none':
        return;
    }
  };

  // The server withholds the raw `strava` / `garmin` rows a mirror card
  // covers and coalesces a card's two backends (carnet#255, carnet#574).
  // Onboarding offers only what an athlete connects themselves: the OAuth,
  // scrape and API-key cards, not the synthetic dev providers. The seat step
  // offers the coaching platforms alone, in seat order.
  const visibleProviders = seat
    ? COACH_PLATFORM_PROVIDERS.flatMap((name) => providers.filter((p) => p.provider === name))
    : providers.filter(
        (p) => p.requires_oauth || p.provider.startsWith('sciotte') || p.provider === 'intervals_icu',
      );
  // Tes données leads with Garmin, Strava and Intervals.icu and folds the rest,
  // with the file import, behind « Plus de services » (carnet#1530); a folded
  // row that is connected opens the fold by itself.
  const leadProviders = seat
    ? visibleProviders
    : ONBOARDING_LEAD_PROVIDERS.flatMap((name) => visibleProviders.filter((p) => p.provider === name));
  const moreProviders = seat ? [] : visibleProviders.filter((p) => !ONBOARDING_LEAD_PROVIDERS.includes(p.provider));
  const showMore = moreOpen || moreProviders.some((p) => p.connected);

  const renderProvider = (provider: ExtendedProviderStatus, last: boolean) => {
    const isConnecting = connectingProvider === provider.provider;
    const isConnected = provider.connected;
    const status = cardStatus(provider);
    // A connected grant whose history is still needed opens the sign-in that
    // brings it in (carnet#1472).
    const historySignIn = status === 'history_needed' ? connectPlan(provider).history.signIn : null;
    const exportSource = cardExportSource(provider);

    return (
      <View key={provider.provider} className="flex-row items-center px-4">
        <View className="w-6 items-center mr-3.5">
          <ProviderGlyph providerId={provider.provider} label={provider.display_name} />
        </View>
        {/* The hairline sits on this inner column, so it insets past the glyph to the text. */}
        <View
          className={`flex-1 flex-row items-center min-h-[52px] ${last ? '' : 'border-b border-border-faint'}`}
          style={last ? undefined : { borderBottomWidth: StyleSheet.hairlineWidth }}
        >
          <View className="flex-1 min-w-0 py-2 mr-3">
            <Text className="text-base text-text-primary">{provider.display_name}</Text>
            {seat ? null : (
              <Text
                className="text-sm text-text-secondary mt-0.5"
                numberOfLines={1}
                testID={`provider-subtitle-${provider.provider}`}
              >
                {provider.description}
              </Text>
            )}
            {refusedCard?.card === provider.provider && (
              <Text
                className="text-sm text-error"
                onPress={() => handleConnect(provider)}
                accessibilityRole="button"
                accessibilityLiveRegion="polite"
                testID={`provider-error-${provider.provider}`}
              >
                {refusedConnectLine(t, provider.display_name)}
              </Text>
            )}
            {exportSource && !seat && (
              <Text
                className="text-sm font-medium text-primary self-start"
                onPress={() => router.push(importRouteFor(exportSource, true))}
                accessibilityRole="link"
                testID={`provider-export-${provider.provider}`}
              >
                {t('providers.import.cardLink')}
              </Text>
            )}
          </View>
          {isConnecting ? (
            <ActivityIndicator size="small" color={colors.tokens.primary} testID={`provider-action-${provider.provider}`} />
          ) : isConnected ? (
            <View className="flex-row items-center gap-2">
              <View className="w-2 h-2 rounded-full bg-success" />
              <Text className="text-sm font-medium text-text-secondary">
                {t('app.connected')}
              </Text>
              {historySignIn && (
                <Text
                  className="text-md font-medium text-primary"
                  onPress={() => openSignIn(historySignIn, provider, true)}
                  accessibilityRole="button"
                  accessibilityHint={t(HISTORY_ACTION_KEYS.explainer, { provider: provider.display_name })}
                  testID={`provider-action-${provider.provider}`}
                >
                  {t(HISTORY_ACTION_KEYS.add)}
                </Text>
              )}
            </View>
          ) : (
            <Text
              className="text-md font-medium text-primary"
              onPress={() => handleConnect(provider)}
              accessibilityRole="button"
              accessibilityLabel={t('app.connectProvider', { provider: provider.display_name })}
              testID={`provider-action-${provider.provider}`}
            >
              {t('app.connect')}
            </Text>
          )}
        </View>
      </View>
    );
  };

  if (heldOnArrival.current) return null;

  // Awaiting OAuth consent — covers the gap between
  // openAuthSessionAsync starting and the user returning from the in-app
  // browser. Cancel button drops back to the cards so they're never trapped.
  if (awaitingOAuthFor) {
    return (
      <SafeAreaView className="flex-1 bg-background-primary items-center justify-center px-8">
        <ActivityIndicator size="large" color={colors.tokens.primary} />
        <Text className="mt-4 text-base font-semibold text-text-primary text-center">
          {t('onboarding.awaitingProviderConsent', { provider: awaitingOAuthFor })}
        </Text>
        <Text className="mt-2 text-sm text-text-tertiary text-center">
          {t('app.finishAuthInBrowser', { provider: awaitingOAuthFor })}
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
          <OnboardingProgressBar steps={progress} />
          <Text
            className="mt-4 text-3xl font-display text-left text-text-primary mb-3"
            accessibilityRole="header"
          >
            {seat ? t('humanCoach.seatHeading') : t('onboarding.connect.heading')}
          </Text>
          <Text className="text-base text-text-secondary leading-6">
            {seat ? t('humanCoach.seatIntro') : t('onboarding.connect.intro')}
          </Text>
        </View>

        {isLoading ? (
          <View className="py-12 items-center">
            <ActivityIndicator size="large" color={colors.tokens.primary} />
          </View>
        ) : (
          <View>
            {leadProviders.map((provider, index) =>
              renderProvider(provider, index === leadProviders.length - 1 && moreProviders.length === 0),
            )}
            {moreProviders.length > 0 ? (
              <Pressable
                onPress={() => setMoreOpen((open) => !open)}
                accessibilityRole="button"
                accessibilityState={{ expanded: showMore }}
                className="flex-row items-center px-4 min-h-[52px]"
                testID="provider-more-services"
              >
                <View className="w-6 items-center mr-3.5">
                  <Feather name={showMore ? 'chevron-down' : 'chevron-right'} size={18} color={colors.text.secondary} />
                </View>
                <View className="flex-1 min-w-0 py-2">
                  <Text className="text-base text-text-primary">{t('onboarding.connect.moreServices')}</Text>
                  {showMore ? null : (
                    <Text className="text-sm text-text-secondary mt-0.5" numberOfLines={1}>
                      {[
                        ...moreProviders.slice(0, 3).map((p) => p.display_name),
                        t('onboarding.connect.moreServicesFile'),
                      ].join(', ')}
                    </Text>
                  )}
                </View>
              </Pressable>
            ) : null}
            {showMore
              ? moreProviders.map((provider, index) => renderProvider(provider, index === moreProviders.length - 1))
              : null}
            {!seat && (showMore || moreProviders.length === 0) && <ImportOwnDataRow onboarding />}
          </View>
        )}

        {connectError && (
          <View
            accessibilityLiveRegion="polite"
            className="mt-4 rounded-lg border border-error/40 bg-error/10 px-4 py-3"
          >
            <Text className="text-sm text-error">{connectError}</Text>
          </View>
        )}

        {seat ? (
          <CoachSeatPanel
            providers={providers}
            onReloadProviders={reloadProviders}
            onDone={() => void markSeatDone()}
          />
        ) : null}

        {seat ? null : anyConnected ? (
          <View className="mt-6" testID="onboarding-connect-continue">
            <Text className="text-sm text-text-secondary text-center mb-3">
              {t('onboarding.connectAnotherOrContinue')}
            </Text>
            <Button title={t('onboarding.continueAfterConnect')} onPress={continuePastConnect} />
          </View>
        ) : (
          <View className="mt-6">
            {/* A visible secondary, saved on the server: the screen never comes back. */}
            <Button
              title={t('onboarding.connect.continueWithout')}
              variant="secondary"
              onPress={skip}
              testID="onboarding-connect-skip"
            />
          </View>
        )}

        {seat ? null : (
          <>
            <Text className="text-xs text-text-tertiary text-center mt-6">
              {t('app.credsEncrypted')}
            </Text>
            <Pressable
              onPress={() => void logout()}
              accessibilityRole="button"
              className="mt-4 min-h-11 items-center justify-center self-center px-4"
              testID="onboarding-logout-link"
            >
              <Text className="text-sm text-text-secondary">{t('common.logout')}</Text>
            </Pressable>
          </>
        )}
      </ScrollView>

      <SciotteLoginModal
        visible={signIn.target !== null}
        onClose={signIn.close}
        onConnected={(followOn) => {
          signIn.close();
          void finalizeConnection();
          // Garmin signs in first and authorizes its grant, which pushes
          // workouts, right after (carnet#1494).
          if (followOn) void launchOAuth(followOn, cardForSlug(followOn, providers)?.display_name ?? followOn);
        }}
        target={signIn.target ?? 'strava'}
        consentRequired={signIn.consentRequired}
        historyMode={signIn.historyMode}
        onFallBackToOAuth={signIn.fallback ? fallBackToOAuth(signIn.fallback) : undefined}
      />

      <IntervalsIcuLinkModal
        visible={intervalsSheet !== null}
        onClose={() => setIntervalsSheet(null)}
        onAuthorize={
          intervalsSheet?.offersAuthorize
            ? () => {
                const { displayName } = intervalsSheet;
                setIntervalsSheet(null);
                void launchOAuth('intervals_icu', displayName);
              }
            : undefined
        }
        onConnected={() => {
          setIntervalsSheet(null);
          void finalizeConnection();
        }}
      />

      <ProviderNoticeSheet
        provider={noticeFor?.provider ?? null}
        onCancel={() => setNoticeFor(null)}
        onAccept={() => {
          const accepted = noticeFor;
          setNoticeFor(null);
          if (accepted) continueConnect(accepted, true);
        }}
      />

      {ownAppSetup && (
        <OAuthAppSetupModal
          visible
          onClose={() => setOwnAppSetup(null)}
          onSaved={() => {
            const { provider, tosConsent } = ownAppSetup;
            setOwnAppSetup(null);
            // BYO credentials are persisted; kick off the standard OAuth dance.
            // launchOAuth handles success → finalizeConnection.
            void launchOAuth(provider.provider, provider.display_name, tosConsent);
          }}
          provider={ownAppSetup.provider.provider}
          displayName={ownAppSetup.provider.display_name}
          devPortalUrl={ownAppSetup.devPortalUrl}
          callbackUrl={ownAppSetup.provider.oauth_callback_url}
        />
      )}
    </SafeAreaView>
  );
}
