// ABOUTME: Hook for managing OAuth provider connection status
// ABOUTME: Handles provider loading, connection checks, and OAuth flow initiation

import { useState, useCallback, useEffect } from 'react';
import { Alert, AppState } from 'react-native';
import * as Linking from 'expo-linking';
import * as WebBrowser from 'expo-web-browser';
import { getOAuthCallbackUrl } from '../../utils/oauth';
import { oauthApi } from '../../services/api';
import { trackMobile } from '../../services/analytics';
import type { ExtendedProviderStatus } from '../../types';
import { useTranslation } from '@pierre/i18n';

export interface ProviderStatusState {
  connectedProviders: ExtendedProviderStatus[];
  /**
   * True once a status call has answered. The list starts empty, so anything
   * derived from it before then reads as "nothing connected" for a connected
   * athlete — which is how the header would flash the wrong line on every open.
   */
  providersLoaded: boolean;
  selectedProvider: string | null;
  providerModalVisible: boolean;
  connectingProvider: string | null;
  needsCredentialsProvider: string | null;
  error: string | null;
}

export interface ProviderStatusActions {
  loadProviderStatus: () => Promise<void>;
  hasConnectedProvider: () => boolean;
  setSelectedProvider: (provider: string | null) => void;
  setProviderModalVisible: (visible: boolean) => void;
  setNeedsCredentialsProvider: (provider: string | null) => void;
  handleConnectProvider: (
    provider: string,
    onSuccess?: () => Promise<void>
  ) => Promise<void>;
  getCachedConnectedProvider: () => ExtendedProviderStatus | undefined;
}

export function useProviderStatus(): ProviderStatusState & ProviderStatusActions {
  const { t } = useTranslation();
  const [connectedProviders, setConnectedProviders] = useState<ExtendedProviderStatus[]>([]);
  const [providersLoaded, setProvidersLoaded] = useState(false);
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const [providerModalVisible, setProviderModalVisible] = useState(false);
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
        err instanceof Error ? err.message : t('providers.failedLoadProviderStatus');
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
    onSuccess?: () => Promise<void>
  ) => {
    setConnectingProvider(provider);
    setError(null);
    try {
      const returnUrl = getOAuthCallbackUrl();
      const oauthResponse = await oauthApi.initMobileOAuth(provider, returnUrl);

      // Dismiss modal only after OAuth URL is ready and browser is about to open
      setProviderModalVisible(false);
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
        err instanceof Error ? err.message : t('providers.failedConnectProvider');

      // Detect missing OAuth credentials — show credential entry instead of error
      const isCredentialError = errorMessage.toLowerCase().includes('client id not configured')
        || errorMessage.toLowerCase().includes('client credentials not configured')
        || errorMessage.toLowerCase().includes('configuration');

      if (isCredentialError) {
        setProviderModalVisible(false);
        setNeedsCredentialsProvider(provider);
      } else {
        setError(errorMessage);
        console.error('Failed to start OAuth:', err);
        Alert.alert(t('common.error'), t('providers.failedConnectRetry'));
      }
    }
  }, [loadProviderStatus, t]);

  return {
    connectedProviders,
    providersLoaded,
    selectedProvider,
    providerModalVisible,
    connectingProvider,
    needsCredentialsProvider,
    error,
    loadProviderStatus,
    hasConnectedProvider,
    setSelectedProvider,
    setProviderModalVisible,
    setNeedsCredentialsProvider,
    handleConnectProvider,
    getCachedConnectedProvider,
  };
}
