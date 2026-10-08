// ABOUTME: Hook for managing OAuth provider connection status
// ABOUTME: Handles provider loading, connection checks, and OAuth flow initiation

import { useState, useCallback, useEffect } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { Alert, AppState } from 'react-native';
import * as Linking from 'expo-linking';
import * as WebBrowser from 'expo-web-browser';
import { getOAuthCallbackUrl } from '../../utils/oauth';
import { oauthApi } from '../../services/api';
import { trackMobile } from '../../services/analytics';
import type { ExtendedProviderStatus } from '../../types';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';

export interface ProviderStatusState {
  connectedProviders: ExtendedProviderStatus[];
  /**
   * True once a status call has answered. The list starts empty, so anything
   * derived from it before then reads as "nothing connected" for a connected
   * athlete — which is how the header would flash the wrong line on every open.
   */
  providersLoaded: boolean;
  selectedProvider: string | null;
  connectingProvider: string | null;
  needsCredentialsProvider: string | null;
  error: string | null;
}

export interface ProviderStatusActions {
  loadProviderStatus: () => Promise<void>;
  hasConnectedProvider: () => boolean;
  setSelectedProvider: (provider: string | null) => void;
  setNeedsCredentialsProvider: (provider: string | null) => void;
  /**
   * Start `provider`'s OAuth flow. `tosConsent` carries the provider notice
   * the athlete just accepted (WHOOP's owner authorization).
   */
  handleConnectProvider: (
    provider: string,
    onSuccess?: () => Promise<void>,
    tosConsent?: boolean
  ) => Promise<void>;
  getCachedConnectedProvider: () => ExtendedProviderStatus | undefined;
}

export function useProviderStatus(): ProviderStatusState & ProviderStatusActions {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [connectedProviders, setConnectedProviders] = useState<ExtendedProviderStatus[]>([]);
  const [providersLoaded, setProvidersLoaded] = useState(false);
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);
  const [needsCredentialsProvider, setNeedsCredentialsProvider] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadProviderStatus = useCallback(async () => {
    try {
      setError(null);
      const response = await oauthApi.getProvidersStatus();
      setConnectedProviders(response.providers || []);
      setProvidersLoaded(true);
    } catch (err) {
      const errorMessage =
        describeApiError(err, { t, fallbackKey: 'providers.failedLoadProviderStatus' });
      setError(errorMessage);
      console.error('Failed to load provider status:', err);
    }
  }, [t]);

  // Refresh provider status when app returns from OAuth flow
  useEffect(() => {
    const subscription = AppState.addEventListener('change', (nextAppState) => {
      if (nextAppState === 'active') {
        loadProviderStatus();
      }
    });
    return () => subscription.remove();
  }, [loadProviderStatus, t]);

  const hasConnectedProvider = useCallback((): boolean => {
    return connectedProviders.some(p => p.connected);
  }, [connectedProviders]);

  const getCachedConnectedProvider = useCallback((): ExtendedProviderStatus | undefined => {
    if (selectedProvider) {
      const cached = connectedProviders.find(
        p => p.provider === selectedProvider && p.connected
      );
      if (cached) return cached;
    }
    return connectedProviders.find(p => p.connected);
  }, [connectedProviders, selectedProvider]);

  const handleConnectProvider = useCallback(async (
    provider: string,
    onSuccess?: () => Promise<void>,
    tosConsent = false
  ) => {
    // No app of the server's can authorize the provider for this athlete:
    // their own app's credentials come first.
    if (connectedProviders.find((p) => p.provider === provider)?.own_app_required) {
      setNeedsCredentialsProvider(provider);
      return;
    }
    setConnectingProvider(provider);
    setError(null);
    try {
      const returnUrl = getOAuthCallbackUrl();
      const oauthResponse = await oauthApi.initMobileOAuth(provider, returnUrl, { tosConsent });

      // The connecting state ends once the OAuth URL is ready and the browser is about to open
      setConnectingProvider(null);

      const result = await WebBrowser.openAuthSessionAsync(
        oauthResponse.authorization_url,
        returnUrl
      );

      if (result.type === 'success' && result.url) {
        const expectedPrefix = getOAuthCallbackUrl();
        if (!result.url.startsWith(expectedPrefix)) {
          console.error('OAuth callback URL does not match expected scheme:', result.url);
          setError(t('app.unexpectedOauthCallback'));
          Alert.alert(t('app.connectionFailed'), t('app.unexpectedOauthCallback'));
          return;
        }

        const parsedUrl = Linking.parse(result.url);
        const success = parsedUrl.queryParams?.success === 'true';
        const errorParam = parsedUrl.queryParams?.error as string | undefined;

        if (success) {
          trackMobile({ name: 'feature_engaged', props: { feature: 'provider_connected' } });
          // The shared status the reconnect banner and the thread header read
          // is asked again too, so a reconnect made from a reply clears both.
          // Invalidated rather than written from this read: a slower read of
          // this hook's, started before the reconnect, could land after it and
          // put the flag back.
          void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() });
          await loadProviderStatus();
          setSelectedProvider(provider);
          if (onSuccess) {
            await onSuccess();
          }
        } else if (errorParam) {
          const reason = t('providers.failedToConnectReason', { reason: errorParam });
          setError(reason);
          console.error('OAuth error from server:', errorParam);
          Alert.alert(t('app.connectionFailed'), reason);
        } else {
          void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() });
          await loadProviderStatus();
          Alert.alert(
            t('providers.connectionComplete'),
            t('providers.connectionFlowCompleted', { provider }),
          );
        }
      } else if (result.type === 'cancel') {
        console.log('OAuth cancelled by user');
      }
    } catch (err) {
      setConnectingProvider(null);
      const errorMessage =
        describeApiError(err, { t, fallbackKey: 'providers.failedConnectProvider' });
      setError(errorMessage);
      console.error('Failed to start OAuth:', err);
      Alert.alert(t('common.error'), t('providers.failedConnectRetry'));
    }
  }, [connectedProviders, loadProviderStatus, queryClient, t]);

  return {
    connectedProviders,
    providersLoaded,
    selectedProvider,
    connectingProvider,
    needsCredentialsProvider,
    error,
    loadProviderStatus,
    hasConnectedProvider,
    setSelectedProvider,
    setNeedsCredentialsProvider,
    handleConnectProvider,
    getCachedConnectedProvider,
  };
}
