// ABOUTME: The Connections pane — one row per fitness data provider, its brand glyph, one status word and one ink action, then the connected-apps section
// ABOUTME: Runs each row's connect plan (authorization, sign-in, Intervals.icu), the history sign-in and the disconnect confirm; a long-press opens the menu

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  Pressable,
  StyleSheet,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  AppState,
  Modal,
} from 'react-native';
import * as WebBrowser from 'expo-web-browser';
import * as Linking from 'expo-linking';
import { useFocusEffect, useRouter } from 'expo-router';
import { getOAuthCallbackUrl } from '../../utils/oauth';
import { promptImportTombstoneClear } from '../../utils/importTombstone';
import { spacing, useThemeColors } from '../../constants/theme';
import { EmptyState, PaneScrollView, Section, Sheet, StatusDot } from '../../components/ui';
import { SciotteLoginModal } from '../../components/SciotteLoginModal';
import { IntervalsIcuLinkModal } from '../../components/IntervalsIcuLinkModal';
import { OAuthCredentialsSection } from '../../components/OAuthCredentialsSection';
import { OAuthAppSetupModal } from '../../components/OAuthAppSetupModal';
import { ProviderNoticeSheet } from '../../components/ProviderNotice';
import { oauthApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useAiConsent } from '../../hooks/useAiConsent';
import type { ExtendedProviderStatus } from '../../types';
import { useTranslation } from '@pierre/i18n';
import type { SciotteTarget } from '@pierre/shared-types';
import {
  AI_CONSENT_KEYS,
  HISTORY_ACTION_KEYS,
  HISTORY_IMPORTING_REFRESH_MS,
  HISTORY_LINE_KEYS,
  OAUTH_NO_SEAT,
  cardForSlug,
  cardHistoryState,
  cardStatus,
  coachPlatformName,
  connectPlan,
  credentialFallbackOnRefusal,
  delegationRefusalKey,
  historyActionKey,
  historyImporting,
  noticeRequired,
  refusalReloadsCards,
  refusalReturnsToCard,
  sciotteTargetForBackend,
} from '@pierre/shared-constants';
import { presentProviderMenu } from './presentProviderMenu';
import { ProviderGlyph } from '../../components/ProviderGlyph';
import { ImportOwnDataRow } from '../../components/ImportOwnDataRow';
import { CONNECTED_APPS_ROUTE } from '../../navigation/routes';
import { PreferredCalendarSection } from './PreferredCalendarSection';
import { useSignInModal } from '../../hooks/useSignInModal';
import { importRouteFor, refusedConnectLine, type RefusedConnect } from '../../utils/connectFlow';
import {
  cardAwaitingHistory,
  cardExportSource,
  describeApiError,
  importTombstoneFromReturn,
  refusalReason,
} from '@pierre/ui-logic';
import { ownAppDevPortal } from '@pierre/domain-utils';

export function ConnectionsScreen() {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const router = useRouter();
  const { isAuthenticated } = useAuth();
  const aiConsent = useAiConsent();
  const [providers, setProviders] = useState<ExtendedProviderStatus[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The provider sign-in window: the account, the notice its row owes, and
  // whether it copies the history of a row already connected.
  const signIn = useSignInModal();
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
  const [showCredentials, setShowCredentials] = useState(false);
  // A provider no app of the server's can authorize for this athlete
  // (`own_app_required`) asks for an app of their own first: the
  // provider-aware setup sheet opens in place, so first-touch users never
  // need to navigate elsewhere, and saving it starts the OAuth flow with the
  // notice acceptance that led here. Mirrors the web provider cards.
  const [ownAppSetup, setOwnAppSetup] = useState<{
    provider: ExtendedProviderStatus;
    devPortalUrl: string;
    tosConsent: boolean;
  } | null>(null);
  // The OAuth provider whose notice is on screen before its flow starts
  // (WHOOP, until the account accepts its owner authorization).
  const [noticeFor, setNoticeFor] = useState<ExtendedProviderStatus | null>(null);
  // Tracks the "Connected!" state shown after a successful OAuth completes.
  // Replaces the legacy Alert.alert success dialog and lines the UX up with
  // the web onboarding screen.
  const [justConnected, setJustConnected] = useState<string | null>(null);

  // Returns the providers it read (none when the read failed), so a connect
  // that just came back can judge the row it landed on. A `quiet` read (the
  // refresh while a history copies) leaves the list on screen.
  const loadConnectionStatus = useCallback(async (
    { quiet = false }: { quiet?: boolean } = {},
  ): Promise<ExtendedProviderStatus[]> => {
    try {
      if (!quiet) {
        setIsLoading(true);
        setError(null);
      }
      const response = await oauthApi.getProvidersStatus();
      const served = response.providers || [];
      setProviders(served);
      return served;
    } catch (err) {
      // A quiet refresh that fails keeps the list it already shows; the next
      // tick reads again.
      if (!quiet) setError(describeApiError(err, { t, fallbackKey: 'app.failedLoadConnections' }));
      console.error('Failed to load connection status:', err);
      // Don't show alert on auth errors - screen will reload when auth is ready
      return [];
    } finally {
      if (!quiet) setIsLoading(false);
    }
  }, [t]);

  /**
   * Tell the row `cardKey` its connect did not go through. When the
   * refusal says the row's plan changed under the athlete (the seat it was
   * shown went), the rows are read again first, and a row that now signs in
   * opens its sign-in at once.
   */
  const showRefusal = async (
    cardKey: string,
    { reload }: { reload: boolean },
  ) => {
    if (reload) {
      const served = await loadConnectionStatus({ quiet: true });
      const card = served.find((p) => p.provider === cardKey);
      const primary = card ? connectPlan(card).primary : null;
      if (card && primary?.kind === 'sign_in') {
        signIn.open(primary.target, card);
        return;
      }
    }
    setRefusedCard({ card: cardKey });
  };

  useEffect(() => {
    if (isAuthenticated) {
      loadConnectionStatus();
    }
  }, [isAuthenticated, loadConnectionStatus]);

  // A row copying its history is read again, while this screen is in view and
  // the app is not in the background, until the copy settles (carnet#1386).
  const importing = historyImporting(providers);
  useFocusEffect(
    useCallback(() => {
      if (!importing) return undefined;
      const timer = setInterval(() => {
        if (AppState.currentState !== 'background') void loadConnectionStatus({ quiet: true });
      }, HISTORY_IMPORTING_REFRESH_MS);
      return () => clearInterval(timer);
    }, [importing, loadConnectionStatus]),
  );

  // Safety net: auto-clear the post-connect spinner after 6 seconds so the
  // user is never pinned on it. The connection-status refetch normally
  // resolves the visible state long before this fires; the timeout just
  // prevents a hung state if anything stalls.
  useEffect(() => {
    if (!justConnected) return undefined;
    const timer = setTimeout(() => setJustConnected(null), 6000);
    return () => clearTimeout(timer);
  }, [justConnected]);

  const handleConnect = async (providerId: string, providerName: string, tosConsent = false) => {
    // The row the authorization belongs to: `garmin` and `strava` land on the
    // card of their sign-in backend, whose row shows the spinner.
    const cardKey = cardForSlug(providerId, providers)?.provider ?? providerId;
    try {
      setConnectingProvider(cardKey);

      // Create return URL for the mobile app (deep link)
      // Server will redirect to this URL after OAuth completes
      // Uses custom scheme (dravr://) for consistent behavior in dev and prod
      const returnUrl = getOAuthCallbackUrl();

      // Call the mobile OAuth init endpoint which returns the authorization URL
      // and includes the redirect URL in the OAuth state for callback handling
      const oauthResponse = await oauthApi.initMobileOAuth(providerId, returnUrl, { tosConsent });

      // Open OAuth in an in-app browser (ASWebAuthenticationSession on iOS)
      // The returnUrl is watched for redirects to close the browser automatically
      const result = await WebBrowser.openAuthSessionAsync(
        oauthResponse.authorization_url,
        returnUrl
      );

      if (result.type === 'success' && result.url) {
        // Validate the callback URL matches our expected scheme/host before processing
        const expectedPrefix = getOAuthCallbackUrl();
        if (!result.url.startsWith(expectedPrefix)) {
          console.error('OAuth callback URL does not match expected scheme:', result.url);
          Alert.alert(t('app.connectionFailed'), t('app.unexpectedOauthCallback'));
          return;
        }

        // Parse the return URL to check for success/error
        const parsedUrl = Linking.parse(result.url);

        // A provider account a deleted Dravr account held: offered to start
        // fresh, never sent to another way in that would be refused the same.
        const tombstone = importTombstoneFromReturn((name) => parsedUrl.queryParams?.[name]);
        if (tombstone) {
          promptImportTombstoneClear({
            refusal: tombstone,
            providerName,
            language,
            t,
            onCleared: () => void handleConnect(providerId, providerName, tosConsent),
          });
          return;
        }

        const success = parsedUrl.queryParams?.success === 'true';
        const error = parsedUrl.queryParams?.error as string | undefined;

        if (success) {
          // OAuth completed successfully — refresh status and show the
          // post-connect spinner. Mirrors the web onboarding UX so the
          // moment of triumph isn't a modal alert the user has to dismiss.
          // A grant whose history is still needed opens the sign-in that
          // copies it instead, at once (carnet#1472).
          const served = await loadConnectionStatus();
          const owed = cardAwaitingHistory(served, providerId);
          if (owed) {
            signIn.open(owed.target, owed.card, true);
          } else {
            setJustConnected(providerName);
          }
        } else if (error) {
          console.error('OAuth error from server:', error);
          // A refused Strava grant falls back to the Strava sign-in (same
          // data). A Garmin grant refused because the seats ran out reads the
          // rows again and follows the plan they now give (the sign-in while
          // the seats are out); any other refused Garmin grant says so on its
          // row, whose tap tries the authorization again.
          const fallback = credentialFallbackOnRefusal(providerId);
          if (fallback) {
            signIn.openFor(fallback, providers);
          } else if (refusalReturnsToCard(providerId)) {
            await showRefusal(cardKey, { reload: error === OAUTH_NO_SEAT });
          } else {
            Alert.alert(t('app.connectionFailed'), t('app.failedToConnectReason', { reason: error }));
          }
        } else {
          // No explicit success/error in the callback — refresh status to
          // see whether the row landed anyway, and surface the spinner if so.
          await loadConnectionStatus();
          setJustConnected(providerName);
        }
      } else if (result.type === 'cancel') {
        console.log('OAuth cancelled by user');
      }
    } catch (err) {
      const errorMessage = describeApiError(err, { t, fallbackKey: 'app.failedToConnect' });
      // Couldn't start the OAuth flow at all. For Strava this also covers the
      // platform Strava app being unconfigured — fall back to the Strava
      // sign-in rather than bouncing to the BYO-credentials sheet or an error
      // alert. Garmin says so on its row; a start refused because its seats
      // changed reads the rows again, and a row that now signs in opens it.
      const fallback = credentialFallbackOnRefusal(providerId);
      if (fallback) {
        console.error('Failed to start the OAuth flow; falling back to the sign-in:', err);
        signIn.openFor(fallback, providers);
        return;
      }
      if (refusalReturnsToCard(providerId)) {
        console.error('Failed to start the OAuth flow:', err);
        await showRefusal(cardKey, { reload: refusalReloadsCards(refusalReason(err)) });
        return;
      }
      const isCredentialError = errorMessage.toLowerCase().includes('client id not configured')
        || errorMessage.toLowerCase().includes('client credentials not configured')
        || errorMessage.toLowerCase().includes('configuration');

      if (isCredentialError) {
        setShowCredentials(true);
      } else {
        console.error('Failed to start OAuth flow:', err);
        Alert.alert(t('common.error'), t('app.failedStartAuth'));
      }
    } finally {
      setConnectingProvider(null);
    }
  };

  const handleDisconnect = async (providerId: string, providerName: string) => {
    Alert.alert(
      t('app.disconnectProvider', { provider: providerName }),
      t('app.confirmDisconnect', { provider: providerName }),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.disconnect'),
          style: 'destructive',
          onPress: async () => {
            try {
              if (providerId === 'intervals_icu') {
                await oauthApi.disconnectIntervalsIcu();
              } else {
                await oauthApi.disconnectProvider(providerId);
              }
              await loadConnectionStatus();
              Alert.alert(t('common.success'), t('app.providerDisconnected', { provider: providerName }));
            } catch (error) {
              console.error('Failed to disconnect provider:', error);
              Alert.alert(t('common.error'), t('app.failedDisconnect', { provider: providerName }));
            }
          },
        },
      ]
    );
  };

  /**
   * Withdraw the consent to AI use of a provider's data in one call
   * (carnet#726), through the same mutation as the Privacy switch so both
   * read the server's answer. The connection stays; the row then asks for
   * the authorization again, which gives it back.
   */
  const handleWithdrawAiConsent = async (providerId: string, providerName: string) => {
    try {
      await aiConsent.mutateAsync({ provider: providerId, allow: false });
      await loadConnectionStatus();
      Alert.alert(t('common.success'), t(AI_CONSENT_KEYS.withdrawn, { provider: providerName }));
    } catch (err) {
      console.error('Failed to withdraw AI consent:', err);
      Alert.alert(t('common.error'), t(AI_CONSENT_KEYS.failed));
    }
  };

  /**
   * The connect flow a provider row starts, for a first connection, a
   * reconnect and the move to the official connection alike: the row's
   * `connectPlan` decides it. The Strava and Garmin rows go through their
   * provider's authorization while the server recommends it (a seat is
   * free, or the row asks to move) and sign in otherwise; a refused Strava
   * authorization falls back to the sign-in, a refused Garmin one says so on
   * the row. COROS and TrainingPeaks sign in, TrainingPeaks after its notice
   * while the account has not accepted it. Whoop is BYO: the setup sheet
   * opens first and OAuth fires once the user saves valid client_id/secret,
   * which spares first-touch users a speculative attempt and its
   * "Configuration error" toast. Intervals.icu goes the way its row says: an
   * authorized row back to the authorization, a key-linked row (a coach's:
   * only the key reads the athletes they coach) to the key form, and a new
   * link to the sheet that leads with the authorization and keeps the key
   * form beside it.
   */
  const startConnect = (provider: ExtendedProviderStatus) => {
    setRefusedCard(null);
    // An OAuth provider whose notice the account has not accepted (WHOOP's
    // owner authorization) states it first; its Continue resumes here.
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
    void handleConnect(provider, card?.display_name ?? provider);
  };

  /** The connect `startConnect` resumes once any OAuth notice is accepted. */
  const continueConnect = (provider: ExtendedProviderStatus, tosConsent: boolean) => {
    const primary = connectPlan(provider).primary;
    switch (primary.kind) {
      case 'sign_in':
        signIn.open(primary.target, provider, false, true);
        return;
      case 'intervals':
        if (primary.path === 'authorize') {
          void handleConnect(provider.provider, provider.display_name, tosConsent);
        } else {
          setIntervalsSheet({ displayName: provider.display_name, offersAuthorize: primary.path === 'choose' });
        }
        return;
      case 'oauth': {
        const devPortalUrl = ownAppDevPortal(provider.provider);
        if (provider.own_app_required && devPortalUrl) {
          setOwnAppSetup({ provider, devPortalUrl, tosConsent });
        } else {
          void handleConnect(primary.provider, provider.display_name, tosConsent);
        }
        return;
      }
      case 'none':
        return;
    }
  };

  const renderProvider = (provider: ExtendedProviderStatus, last: boolean) => {
    const id = provider.provider;
    const isConnected = provider.connected;
    // A connected-but-dead session (dead sciotte scrape / failed OAuth
    // refresh): the row says "Expiré" and its action reconnects, instead of a
    // healthy-looking "Connecté" with only a disconnect affordance.
    // What the row shows, from the one decision web reads too: a WHOOP that
    // owes its owner authorization has stopped syncing and asks for it; a
    // row served by a sign-in alone asks to move to the official connection.
    const status = cardStatus(provider);
    const needsReauth = status === 'reconnect';
    const owesAuthorization = status === 'authorize_owed';
    const isConnecting = connectingProvider === id;
    const canConnect = provider.requires_oauth || id.startsWith('sciotte') || id === 'intervals_icu';
    const hasMenu = isConnected && canConnect;

    // The row's one action as an ink word; the spinner takes its place while
    // the connect flow this row started is in flight.
    const action = (label: string, onPress: () => void, hint?: string) =>
      isConnecting ? (
        <ActivityIndicator size="small" color={colors.tokens.primary} testID={`provider-action-${id}`} />
      ) : (
        <Text
          className="text-md font-medium text-primary"
          onPress={onPress}
          accessibilityRole="button"
          accessibilityHint={hint}
          testID={`provider-action-${id}`}
        >
          {label}
        </Text>
      );
    const reconnect = () => startConnect(provider);
    const disconnect = () => handleDisconnect(id, provider.display_name);
    // A connection the athlete's group coach serves: confirmed, it is read
    // through the coach's account and disconnecting it ends the link;
    // proposed, it waits for the athlete's answer in the group.
    const delegation = provider.delegation;
    const isDelegated = isConnected && delegation?.status === 'confirmed';
    // Where the row's past activities stand: a line under the subtitle. An
    // export the server offers for the row links to its import, connected or
    // not.
    const historyState = cardHistoryState(provider);
    const exportSource = cardExportSource(provider);
    let subtitle = provider.description;
    if (owesAuthorization) {
      subtitle = t('providers.authorizeToKeepSyncing', { provider: provider.display_name });
    } else if (isDelegated) {
      if (delegation.read_refused) {
        subtitle = t(delegationRefusalKey(delegation.read_refused), { platform: coachPlatformName(id) });
      } else {
        subtitle = delegation.coach_needs_reauth
          ? t('delegation.coachReconnectNeeded', {
              platform: coachPlatformName(id),
              coach: delegation.coach_display_name,
            })
          : t('providers.connectedThrough', { coach: delegation.coach_display_name });
      }
    } else if (delegation?.status === 'proposed') {
      subtitle = t('providers.pendingLink', {
        coach: delegation.coach_display_name,
        group: delegation.group_name,
      });
    } else if (provider.account_role === 'coach') {
      subtitle = t('humanCoach.trainingpeaksAccountHint');
    }

    // One status word and one ink action, never a pill or a filled button:
    // the state reads as text and the thing to do about it as a link.
    let trailing: React.ReactNode = null;
    if (isDelegated) {
      trailing = (
        <>
          <StatusDot tone={delegation.coach_needs_reauth || delegation.read_refused ? 'warning' : 'success'} />
          {action(t('delegation.unlink'), disconnect)}
        </>
      );
    } else if (owesAuthorization) {
      trailing = (
        <>
          <StatusDot tone="warning" />
          {action(t('providers.authorizeAction'), reconnect)}
        </>
      );
    } else if (needsReauth) {
      trailing = (
        <>
          <Text className="text-sm font-medium text-warning">{t('providers.expired')}</Text>
          {canConnect && action(t('app.reconnect'), reconnect)}
        </>
      );
    } else if (isConnected) {
      // A push-fed grant (Garmin) carries only what comes after it: while the
      // history is needed, or its last copy did not finish, the row opens the
      // sign-in that copies it; disconnecting stays in the row's menu
      // (carnet#1472).
      const historySignIn =
        status === 'history_needed' || status === 'history_incomplete' ? connectPlan(provider).history.signIn : null;
      trailing = (
        <>
          <StatusDot tone="success" />
          <Text className="text-sm font-medium text-text-secondary">{t('app.connected')}</Text>
          {historySignIn
            ? action(
                t(historyActionKey(provider)),
                () => signIn.open(historySignIn, provider, true),
                t(HISTORY_ACTION_KEYS.explainer, { provider: provider.display_name }),
              )
            : canConnect && action(t('app.disconnect'), disconnect)}
        </>
      );
    } else if (canConnect) {
      trailing = action(t('app.connect'), reconnect);
    }

    // The row pays the pane's 16 inset itself, as a settings `Row` does, so
    // its press target runs to the pane's edge and its hairline does not.
    return (
      <Pressable
        key={id}
        className="flex-row items-center px-4"
        onLongPress={
          hasMenu
            ? () =>
                presentProviderMenu(
                  {
                    providerName: provider.display_name,
                    canReconnect: needsReauth || owesAuthorization,
                    onReconnect: reconnect,
                    onDisconnect: disconnect,
                    disconnectLabel: isDelegated ? t('delegation.unlink') : undefined,
                    onWithdrawAiConsent:
                      provider.ai_consent === true
                        ? () => handleWithdrawAiConsent(id, provider.display_name)
                        : undefined,
                  },
                  t,
                )
            : undefined
        }
        accessibilityRole={hasMenu ? 'button' : undefined}
        testID={`provider-row-${id}`}
      >
        <View className="w-6 items-center mr-3.5">
          <ProviderGlyph providerId={id} label={provider.display_name} />
        </View>
        {/* The hairline sits on this inner column, so it insets past the glyph to the text. */}
        <View
          className={`flex-1 flex-row items-center min-h-[52px] ${last ? '' : 'border-b border-border-faint'}`}
          style={last ? undefined : { borderBottomWidth: StyleSheet.hairlineWidth }}
        >
          <View className="flex-1 min-w-0 py-2">
            <View className="flex-row items-baseline gap-2">
              <Text className="text-base text-text-primary">{provider.display_name}</Text>
              {provider.account_role === 'coach' && (
                <Text className="text-xs font-medium text-text-tertiary" testID={`provider-coach-account-${id}`}>
                  {t('humanCoach.trainingpeaksAccount')}
                </Text>
              )}
            </View>
            <Text
              className="text-sm text-text-secondary"
              numberOfLines={subtitle === provider.description ? 1 : 2}
              testID={`provider-subtitle-${id}`}
            >
              {subtitle}
            </Text>
            {historyState !== 'not_applicable' && (
              <View testID={`provider-history-${id}`}>
                <View className="flex-row items-center gap-1.5">
                  {(historyState === 'needed' || historyState === 'incomplete') && <StatusDot tone="warning" />}
                  <Text
                    className={`text-sm ${historyState === 'imported' ? 'text-text-tertiary' : 'text-text-secondary'}`}
                    accessibilityLiveRegion={historyState === 'importing' ? 'polite' : undefined}
                  >
                    {t(HISTORY_LINE_KEYS[historyState], { provider: provider.display_name })}
                  </Text>
                </View>
              </View>
            )}
            {refusedCard?.card === id && (
              <Text
                className="text-sm text-error"
                onPress={reconnect}
                accessibilityRole="button"
                accessibilityLiveRegion="polite"
                testID={`provider-error-${id}`}
              >
                {refusedConnectLine(t, provider.display_name)}
              </Text>
            )}
            {exportSource && (
              <Text
                className="text-sm font-medium text-primary self-start"
                onPress={() => router.push(importRouteFor(exportSource, false))}
                accessibilityRole="link"
                testID={`provider-export-${id}`}
              >
                {t('providers.import.cardLink')}
              </Text>
            )}
          </View>
          {trailing !== null && <View className="flex-row items-center gap-2 ml-3">{trailing}</View>}
        </View>
      </Pressable>
    );
  };

  // Rendered as served: the server withholds the raw `strava` / `garmin` rows
  // a mirror card covers and coalesces a card's two backends before answering
  // (carnet#255, carnet#574), so no client filters or merges them again.

  // Which providers are connected, in a stable form: the calendar choice reads
  // its calendars again when a connection is made or removed here.
  const connectedProviders = providers
    .filter((provider) => provider.connected)
    .map((provider) => provider.provider)
    .sort()
    .join(',');

  return (
    <View className="flex-1 bg-background-primary" testID="connections-screen">
      <PaneScrollView
        contentContainerStyle={{ paddingVertical: spacing.lg }}
        showsVerticalScrollIndicator={false}
      >
        <View className="gap-8">
          <Section
            title={t('providers.fitnessTitle')}
            description={t('app.connectAccountsBlurb')}
            testID="connections-providers-section"
          >
            {isLoading ? (
              <View className="items-center py-12">
                <ActivityIndicator size="large" color={colors.tokens.primary} />
                <Text className="mt-3 text-text-secondary text-base">{t('app.loadingConnections')}</Text>
              </View>
            ) : error ? (
              // A plain line, so it pays the pane's inset itself.
              <View className="flex-row flex-wrap items-baseline px-4 py-3">
                <Text className="text-sm text-error">{error}</Text>
                <Text
                  className="text-sm text-primary font-medium ml-1"
                  onPress={() => {
                    setError(null);
                    loadConnectionStatus();
                  }}
                  accessibilityRole="button"
                  testID="connections-retry"
                >
                  {t('common.retry')}
                </Text>
              </View>
            ) : (
              <View>
                {providers.map((provider, index) =>
                  renderProvider(provider, index === providers.length - 1),
                )}
              </View>
            )}
            <Text className="text-xs text-text-tertiary px-4 pt-2.5">{t('app.privacyNoteBlurb')}</Text>
            {/* Import your own data: below the providers, never one of them */}
            <ImportOwnDataRow />
          </Section>

          {/* Which connected calendar the plan goes on (carnet#1393) */}
          <PreferredCalendarSection connected={connectedProviders} />

          <Section title={t('tokens.connectedApps')} testID="connections-apps-section">
            <EmptyState
              action={{
                label: t('settingsTabs.tokens'),
                onPress: () => router.push(CONNECTED_APPS_ROUTE),
                testID: 'connections-manage-apps',
              }}
            >
              {t('tokens.connectedAppsEmpty')}
            </EmptyState>
          </Section>
        </View>
      </PaneScrollView>

      <SciotteLoginModal
        visible={signIn.target !== null}
        onClose={signIn.close}
        onConnected={() => {
          loadConnectionStatus();
          signIn.close();
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
                handleConnect('intervals_icu', displayName);
              }
            : undefined
        }
        onConnected={() => {
          loadConnectionStatus();
          setIntervalsSheet(null);
        }}
      />

      <Sheet
        visible={showCredentials}
        onClose={() => setShowCredentials(false)}
        testID="connections-credentials-sheet"
        flush
      >
        <OAuthCredentialsSection providers={providers} />
      </Sheet>

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
            // BYO credentials are persisted; now kick off the standard OAuth
            // dance. handleConnect() will set justConnected on success which
            // surfaces the post-connect spinner.
            void handleConnect(provider.provider, provider.display_name, tosConsent);
          }}
          provider={ownAppSetup.provider.provider}
          displayName={ownAppSetup.provider.display_name}
          devPortalUrl={ownAppSetup.devPortalUrl}
          callbackUrl={ownAppSetup.provider.oauth_callback_url}
        />
      )}

      <Modal
        visible={justConnected !== null}
        animationType="fade"
        transparent
        onRequestClose={() => setJustConnected(null)}
      >
        <View className="flex-1 bg-background-primary items-center justify-center px-8">
          <ActivityIndicator size="large" color={colors.tokens.primary} />
          <Text className="mt-4 text-base font-medium text-text-primary text-center">
            {t('app.connectedBang')}
          </Text>
          <Text className="mt-1 text-sm text-text-secondary text-center">{justConnected}</Text>
          <TouchableOpacity
            className="mt-8 px-6 py-3"
            onPress={() => setJustConnected(null)}
            hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
          >
            <Text className="text-sm text-text-tertiary">{t('app.dismiss')}</Text>
          </TouchableOpacity>
        </View>
      </Modal>
    </View>
  );
}
