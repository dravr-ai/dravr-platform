// ABOUTME: The Connections pane — one row per fitness data provider, its brand glyph, one status word and one ink action, then the connected-apps section
// ABOUTME: Runs the OAuth, Sciotte and Intervals.icu connect flows and the disconnect confirm; a long-press on a connected row opens the platform menu

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  Pressable,
  StyleSheet,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  Modal,
} from 'react-native';
import * as WebBrowser from 'expo-web-browser';
import * as Linking from 'expo-linking';
import { useRouter } from 'expo-router';
import { getOAuthCallbackUrl } from '../../utils/oauth';
import { spacing, useThemeColors } from '../../constants/theme';
import { EmptyState, PaneScrollView, Section, Sheet, StatusDot } from '../../components/ui';
import { SciotteLoginModal } from '../../components/SciotteLoginModal';
import { IntervalsIcuLinkModal } from '../../components/IntervalsIcuLinkModal';
import { OAuthCredentialsSection } from '../../components/OAuthCredentialsSection';
import { OAuthAppSetupModal } from '../../components/OAuthAppSetupModal';
import { ProviderNoticeSheet } from '../../components/ProviderNotice';
import { oauthApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import type { ExtendedProviderStatus } from '../../types';
import { useTranslation } from '@pierre/i18n';
import { noticeRequired, sciotteTargetForBackend, syncAuthorizationOwed } from '@pierre/shared-constants';
import type { SciotteTarget } from '@pierre/shared-types';
import { presentProviderMenu } from './presentProviderMenu';
import { ProviderGlyph } from '../../components/ProviderGlyph';
import { CONNECTED_APPS_ROUTE } from '../../navigation/routes';
import { describeApiError } from '@pierre/ui-logic';

export function ConnectionsScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const router = useRouter();
  const { isAuthenticated } = useAuth();
  const [providers, setProviders] = useState<ExtendedProviderStatus[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sciotteTarget, setSciotteTarget] = useState<SciotteTarget | null>(null);
  // Whether the chosen row still needs its exposure notice accepted.
  const [sciotteConsentRequired, setSciotteConsentRequired] = useState(false);
  const [intervalsModalVisible, setIntervalsModalVisible] = useState(false);
  const [showCredentials, setShowCredentials] = useState(false);
  // Whoop is BYO-OAuth-app: users register their own developer app at
  // developer.whoop.com and paste client_id/secret before the OAuth dance can
  // run. Rather than fall through the generic credentials sheet, open the
  // provider-aware setup sheet in-place so first-touch users never need to
  // navigate elsewhere. Mirrors the web onboarding flow.
  const [showWhoopSetup, setShowWhoopSetup] = useState(false);
  // The OAuth provider whose notice is on screen before its flow starts
  // (WHOOP, until the account accepts its owner authorization).
  const [noticeFor, setNoticeFor] = useState<ExtendedProviderStatus | null>(null);
  // Whether the athlete accepted WHOOP's owner authorization before the
  // setup sheet; the OAuth start after it carries that acceptance.
  const [whoopTosConsent, setWhoopTosConsent] = useState(false);
  // Tracks the "Connected!" state shown after a successful OAuth completes.
  // Replaces the legacy Alert.alert success dialog and lines the UX up with
  // the web onboarding screen.
  const [justConnected, setJustConnected] = useState<string | null>(null);

  const loadConnectionStatus = useCallback(async () => {
    try {
      setIsLoading(true);
      setError(null);
      const response = await oauthApi.getProvidersStatus();
      setProviders(response.providers || []);
    } catch (err) {
      const errorMessage = describeApiError(err, { t, fallbackKey: 'app.failedLoadConnections' });
      setError(errorMessage);
      console.error('Failed to load connection status:', err);
      // Don't show alert on auth errors - screen will reload when auth is ready
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    if (isAuthenticated) {
      loadConnectionStatus();
    }
  }, [isAuthenticated, loadConnectionStatus]);

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
    try {
      setConnectingProvider(providerId);

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
        const success = parsedUrl.queryParams?.success === 'true';
        const error = parsedUrl.queryParams?.error as string | undefined;

        if (success) {
          // OAuth completed successfully — refresh status and show the
          // post-connect spinner. Mirrors the web onboarding UX so the
          // moment of triumph isn't a modal alert the user has to dismiss.
          await loadConnectionStatus();
          setJustConnected(providerName);
        } else if (error) {
          console.error('OAuth error from server:', error);
          // Strava OAuth failed (the shared-app athlete cap was actually
          // exceeded in a seat-count race, or the provider rejected the grant).
          // Fall back to the Sciotte credential login — same Strava data —
          // instead of leaving the user on an alert.
          if (providerId === 'strava') {
            setSciotteTarget('strava');
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
      // platform Strava app being unconfigured — fall back to the Sciotte
      // credential login rather than bouncing to the BYO-credentials sheet or
      // an error alert.
      if (providerId === 'strava') {
        console.error('Failed to start Strava OAuth flow; falling back to Sciotte:', err);
        setSciotteTarget('strava');
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
   * The connect flow a provider row starts, for a first connection and for a
   * reconnect alike. The `sciotte` row is the user-facing Strava row: OAuth is
   * the default while shared-app seats remain (the server recommends `oauth`);
   * once the athlete cap is reached it recommends `mirror` and we go straight
   * to the Sciotte credential login. If the OAuth attempt itself fails,
   * handleConnect falls back to Sciotte. Garmin and TrainingPeaks are always
   * credentials, TrainingPeaks after its notice while the account has not
   * accepted it. Whoop is BYO: the setup sheet opens first and OAuth fires
   * once the user saves valid client_id/secret, which spares first-touch
   * users a speculative attempt and its "Configuration error" toast.
   */
  const startConnect = (provider: ExtendedProviderStatus) => {
    // An OAuth provider whose notice the account has not accepted (WHOOP's
    // owner authorization) states it first; its Continue resumes here.
    if (noticeRequired(provider.provider, provider.consent_required) && !sciotteTargetForBackend(provider.provider)) {
      setNoticeFor(provider);
      return;
    }
    continueConnect(provider, false);
  };

  /** The connect `startConnect` resumes once any OAuth notice is accepted. */
  const continueConnect = (provider: ExtendedProviderStatus, tosConsent: boolean) => {
    const target = sciotteTargetForBackend(provider.provider);
    if (target) {
      if (target === 'strava' && provider.recommended_backend === 'oauth') {
        handleConnect('strava', provider.display_name);
      } else {
        setSciotteConsentRequired(provider.consent_required);
        setSciotteTarget(target);
      }
    } else if (provider.provider === 'intervals_icu') {
      setIntervalsModalVisible(true);
    } else if (provider.provider === 'whoop') {
      setWhoopTosConsent(tosConsent);
      setShowWhoopSetup(true);
    } else {
      handleConnect(provider.provider, provider.display_name, tosConsent);
    }
  };

  // The one line under a provider's name. After the 2026-Q2 provider cleanup
  // the API surfaces `sciotte` (Strava-branded), `sciotte_garmin`
  // (Garmin-branded), `sciotte_trainingpeaks` (TrainingPeaks-branded),
  // `sciotte_coros` (COROS-branded), `whoop` and `intervals_icu`; an unknown id gets the
  // generic line so the screen never crashes on an unexpected payload.
  const providerBlurb = (providerId: string): string => {
    const blurbs: Record<string, string> = {
      sciotte: t('app.provStravaBlurb'),
      sciotte_garmin: t('app.provGarminBlurb'),
      sciotte_trainingpeaks: t('app.provTrainingPeaksBlurb'),
      sciotte_coros: t('app.provCorosBlurb'),
      whoop: t('app.provWhoopBlurb'),
      intervals_icu: t('app.provIntervalsBlurb'),
    };
    return blurbs[providerId] ?? t('app.fitnessDataProvider');
  };

  const renderProvider = (provider: ExtendedProviderStatus, last: boolean) => {
    const id = provider.provider;
    const isConnected = provider.connected;
    // A connected-but-dead session (dead sciotte scrape / failed OAuth
    // refresh): the row says "Expiré" and its action reconnects, instead of a
    // healthy-looking "Connecté" with only a disconnect affordance.
    const needsReauth = provider.connected && provider.needs_reauth;
    // A connected WHOOP that owes its owner authorization has stopped
    // syncing: the row asks for it instead of reading "Connecté".
    const owesAuthorization = syncAuthorizationOwed(id, provider.connected, provider.consent_required);
    const isConnecting = connectingProvider === id;
    const canConnect = provider.requires_oauth || id.startsWith('sciotte') || id === 'intervals_icu';
    const hasMenu = isConnected && canConnect;

    // The row's one action as an ink word; the spinner takes its place while
    // the connect flow this row started is in flight.
    const action = (label: string, onPress: () => void) =>
      isConnecting ? (
        <ActivityIndicator size="small" color={colors.tokens.primary} testID={`provider-action-${id}`} />
      ) : (
        <Text
          className="text-md font-medium text-primary"
          onPress={onPress}
          accessibilityRole="button"
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
    let subtitle = providerBlurb(id);
    if (owesAuthorization) {
      subtitle = t('providers.authorizeToKeepSyncing', { provider: provider.display_name });
    } else if (isDelegated) {
      subtitle = delegation.coach_needs_reauth
        ? t('delegation.coachReconnectNeeded', { coach: delegation.coach_display_name })
        : t('providers.connectedThrough', { coach: delegation.coach_display_name });
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
          <StatusDot tone={delegation.coach_needs_reauth ? 'warning' : 'success'} />
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
      trailing = (
        <>
          <StatusDot tone="success" />
          <Text className="text-sm font-medium text-text-secondary">{t('app.connected')}</Text>
          {canConnect && action(t('app.disconnect'), disconnect)}
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
              numberOfLines={subtitle === providerBlurb(id) ? 1 : 2}
              testID={`provider-subtitle-${id}`}
            >
              {subtitle}
            </Text>
          </View>
          {trailing !== null && <View className="flex-row items-center gap-2 ml-3">{trailing}</View>}
        </View>
      </Pressable>
    );
  };

  // After the 2026-Q2 provider cleanup the API surfaces sciotte,
  // sciotte_garmin, whoop and intervals_icu. The bare `strava` row is hidden:
  // official OAuth is reached exclusively through the Sciotte modal's
  // t('app.useOwnStravaApp') button, so a separate strava row would duplicate
  // the entry, and `connected` already counts either backend behind a row —
  // the server coalesces it (carnet#255). Mirrors
  // frontend/src/components/ProviderConnectionCards.tsx.
  const visibleProviders = providers.filter((p) => p.provider !== 'strava');

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
                {visibleProviders.map((provider, index) =>
                  renderProvider(provider, index === visibleProviders.length - 1),
                )}
              </View>
            )}
            <Text className="text-xs text-text-tertiary px-4 pt-2.5">{t('app.privacyNoteBlurb')}</Text>
          </Section>

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
        visible={sciotteTarget !== null}
        onClose={() => setSciotteTarget(null)}
        onConnected={() => {
          loadConnectionStatus();
          setSciotteTarget(null);
        }}
        target={sciotteTarget ?? 'strava'}
        consentRequired={sciotteConsentRequired}
      />

      <IntervalsIcuLinkModal
        visible={intervalsModalVisible}
        onClose={() => setIntervalsModalVisible(false)}
        onConnected={() => {
          loadConnectionStatus();
          setIntervalsModalVisible(false);
        }}
      />

      <Sheet
        visible={showCredentials}
        onClose={() => setShowCredentials(false)}
        testID="connections-credentials-sheet"
        flush
      >
        <OAuthCredentialsSection />
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

      <OAuthAppSetupModal
        visible={showWhoopSetup}
        onClose={() => setShowWhoopSetup(false)}
        onSaved={() => {
          setShowWhoopSetup(false);
          // BYO credentials are persisted; now kick off the standard OAuth
          // dance. handleConnect() will set justConnected on success which
          // surfaces the post-connect spinner.
          void handleConnect('whoop', 'WHOOP', whoopTosConsent);
        }}
        provider="whoop"
        displayName="WHOOP"
        devPortalUrl="https://developer.whoop.com/"
      />

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
