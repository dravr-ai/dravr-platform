// ABOUTME: First-run onboarding screen that forces a provider connection before the user reaches chat
// ABOUTME: Same OAuth flow as ConnectionsScreen but stripped chrome (no back button) — the user can only exit via OAuth success

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
import { PROVIDER_COLORS, useThemeColors } from '../../constants/theme';
import { Card } from '../../components/ui';
import { oauthApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { getOAuthCallbackUrl } from '../../utils/oauth';
import type { ExtendedProviderStatus } from '../../types';

/**
 * Backed by the same source of truth (`provider_connections`) as the
 * backend's `NoProviderConnected` 403 gate on chat/coach/messaging — so this
 * screen cannot drift from server-side enforcement.
 *
 * Intentionally has NO back button or skip card. The user reaches it only
 * because RootLayoutNav saw `needs_provider_connection: true`; the only way
 * out is to complete OAuth on one of the listed providers, at which point
 * RootLayoutNav re-runs and routes to the chat stack.
 */
export function OnboardingConnectScreen() {
  const colors = useThemeColors();
  const queryClient = useQueryClient();
  const { isAuthenticated, user } = useAuth();
  const [providers, setProviders] = useState<ExtendedProviderStatus[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);

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

  const handleConnect = async (providerId: string, _providerName: string) => {
    try {
      setConnectingProvider(providerId);
      const returnUrl = getOAuthCallbackUrl();
      const oauthResponse = await oauthApi.initMobileOAuth(providerId, returnUrl);
      const result = await WebBrowser.openAuthSessionAsync(oauthResponse.authorization_url, returnUrl);

      if (result.type === 'success' && result.url) {
        if (!result.url.startsWith(returnUrl)) {
          Alert.alert('Connection Failed', 'Unexpected OAuth callback URL');
          return;
        }
        const parsed = Linking.parse(result.url);
        const success = parsed.queryParams?.success === 'true';
        const error = parsed.queryParams?.error as string | undefined;
        if (success) {
          // Invalidate the onboarding-status query so RootLayoutNav re-evaluates
          // and routes the user into the chat stack.
          await queryClient.invalidateQueries({ queryKey: ['user-onboarding-status'] });
        } else if (error) {
          Alert.alert('Connection Failed', `Failed to connect: ${error}`);
        }
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to connect';
      console.error('Onboarding OAuth flow failed:', err);
      Alert.alert('Error', message);
    } finally {
      setConnectingProvider(null);
    }
  };

  const renderProvider = (provider: ExtendedProviderStatus) => {
    // After the 2026-Q2 provider cleanup the API surfaces only three: `sciotte`
    // (Strava-branded), `sciotte_garmin` (Garmin-branded), and `whoop`. Unknown
    // ids fall back to a neutral slate tile.
    const config: Record<string, { color: string; icon: string; description: string }> = {
      sciotte: { color: PROVIDER_COLORS.strava, icon: 'S', description: 'Running, cycling, swimming' },
      sciotte_garmin: { color: PROVIDER_COLORS.garmin, icon: 'G', description: 'Activities + health metrics' },
      whoop: { color: PROVIDER_COLORS.whoop, icon: 'W', description: 'Recovery, strain, sleep' },
    };
    const c = config[provider.provider] ?? { color: '#607D8B', icon: '?', description: 'Fitness data' };
    const isConnecting = connectingProvider === provider.provider;
    if (!provider.requires_oauth) {
      return null;
    }
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
          <TouchableOpacity
            className="px-5 py-2 rounded-full"
            style={{ backgroundColor: c.color }}
            onPress={() => void handleConnect(provider.provider, provider.display_name)}
            disabled={isConnecting}
            activeOpacity={0.7}
            accessibilityLabel={`Connect ${provider.display_name}`}
          >
            {isConnecting ? (
              <ActivityIndicator size="small" color="#FFFFFF" />
            ) : (
              <Text className="text-sm font-semibold text-on-surface">Connect</Text>
            )}
          </TouchableOpacity>
        </View>
      </Card>
    );
  };

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="onboarding-screen">
      <ScrollView contentContainerStyle={{ padding: 20, paddingBottom: 40 }}>
        <View className="mb-6 mt-4">
          <Text
            className="text-3xl font-bold text-text-primary mb-3"
            accessibilityRole="header"
          >
            {user?.display_name ? `Welcome, ${user.display_name}` : 'Welcome to Dravr'}
          </Text>
          <Text className="text-base text-text-secondary leading-6">
            Connect a fitness service to get started. Dravr coaches you on the activities your
            provider already tracks — without one, there's nothing for the model to read.
          </Text>
        </View>

        {isLoading ? (
          <View className="py-12 items-center">
            <ActivityIndicator size="large" color={colors.tokens.primary} />
          </View>
        ) : (
          providers.map(renderProvider)
        )}

        <Text className="text-xs text-text-tertiary text-center mt-6">
          Your credentials are encrypted at rest and used only to fetch your activity data.
        </Text>
      </ScrollView>
    </SafeAreaView>
  );
}
