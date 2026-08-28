// ABOUTME: Provider selection modal for connecting fitness data providers
// ABOUTME: Shows available providers with connection status and OAuth flow initiation

import React, { useMemo } from 'react';
import { View, Text, TouchableOpacity, Modal, ActivityIndicator } from 'react-native';
import type { ViewStyle } from 'react-native';
import { PRIMARY_PALETTE, spacing, borderRadius, useThemeColors } from '../../constants/theme';
import type { ExtendedProviderStatus } from '../../types';
import { useTranslation } from '@pierre/i18n';

// After the 2026-Q2 provider cleanup the API surfaces only three: `sciotte`
// (Strava-branded), `sciotte_garmin` (Garmin-branded), and `whoop`. Unknown ids
// fall through to the link emoji default in the row renderer below.
const PROVIDER_ICONS: Record<string, string> = {
  sciotte: '🚴',
  sciotte_garmin: '⌚',
  whoop: '💪',
  intervals_icu: '📈',
};

interface ProviderModalProps {
  visible: boolean;
  providers: ExtendedProviderStatus[];
  connectingProvider: string | null;
  onClose: () => void;
  onSelectConnected: (provider: string) => void;
  onConnectProvider: (provider: string) => void;
  onConnectSciotte: (target: 'strava' | 'garmin') => void;
  onConnectIntervals: () => void;
}

export function ProviderModal({
  visible,
  providers,
  connectingProvider,
  onClose,
  onSelectConnected,
  onConnectProvider,
  onConnectSciotte,
  onConnectIntervals,
}: ProviderModalProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const providerModalContainerStyle: ViewStyle = useMemo(() => ({
    backgroundColor: colors.background.primary,
    borderRadius: borderRadius.lg,
    padding: spacing.lg,
    minWidth: 280,
    maxWidth: 320,
    shadowColor: colors.text.primary,
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 8,
    elevation: 8,
  }), [colors]);
  return (
    <Modal
      visible={visible}
      animationType="fade"
      transparent
      onRequestClose={onClose}
    >
      <TouchableOpacity
        className="flex-1 bg-black/50 justify-center items-center"
        activeOpacity={1}
        onPress={onClose}
      >
        <View
          style={providerModalContainerStyle}
          onStartShouldSetResponder={() => true}
        >
          <Text className="text-lg font-semibold text-text-primary text-center mb-1">{t('app.connectAProvider')}</Text>
          <Text className="text-sm text-text-secondary text-center mb-6">
            {t('app.connectFirstBlurb')}
          </Text>

          {providers.map((provider) => {
            const icon = PROVIDER_ICONS[provider.provider] || '🔗';
            const isConnected = provider.connected;
            const requiresOAuth = provider.requires_oauth;
            const isSciotte = provider.provider.startsWith('sciotte');
            const isIntervals = provider.provider === 'intervals_icu';
            const isConnectable = isConnected || requiresOAuth || isSciotte || isIntervals;
            const displayName = provider.display_name || provider.provider;
            const isConnecting = connectingProvider === provider.provider;
            const isOtherConnecting = connectingProvider !== null && !isConnecting;

            return (
              <TouchableOpacity
                key={provider.provider}
                className={`flex-row items-center bg-background-secondary rounded-lg p-4 mb-2 border ${
                  isConnected ? 'border-accent-primary' : isConnecting ? 'border-accent-secondary' : 'border-border-default'
                }`}
                onPress={() => {
                  if (isConnected) {
                    onSelectConnected(provider.provider);
                  } else if (isSciotte) {
                    onConnectSciotte(provider.provider === 'sciotte_garmin' ? 'garmin' : 'strava');
                  } else if (isIntervals) {
                    onConnectIntervals();
                  } else if (requiresOAuth) {
                    onConnectProvider(provider.provider);
                  }
                }}
                disabled={!isConnectable || isOtherConnecting || isConnecting}
              >
                {isConnecting ? (
                  <ActivityIndicator size="small" color={PRIMARY_PALETTE[500]} className="mr-4" />
                ) : (
                  <Text className="text-2xl mr-4">{icon}</Text>
                )}
                <View className="flex-1">
                  <Text className={`text-base font-medium ${isOtherConnecting ? 'text-text-tertiary' : 'text-text-primary'}`}>
                    {isConnecting ? t('app.connectingProvider', { provider: displayName }) : isConnected ? displayName : t('app.connectProvider', { provider: displayName })}
                  </Text>
                  {isConnected && (
                    <Text className="text-xs text-accent-primary">{t('app.connectedCheck')}</Text>
                  )}
                </View>
              </TouchableOpacity>
            );
          })}

          <TouchableOpacity
            className="items-center p-4 mt-1"
            onPress={onClose}
          >
            <Text className="text-base text-text-tertiary">{t('common.cancel')}</Text>
          </TouchableOpacity>
        </View>
      </TouchableOpacity>
    </Modal>
  );
}
