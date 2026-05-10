// ABOUTME: Provider selection modal for connecting fitness data providers
// ABOUTME: Shows available providers with connection status and OAuth flow initiation

import React, { useMemo } from 'react';
import { View, Text, TouchableOpacity, Modal, ActivityIndicator } from 'react-native';
import type { ViewStyle } from 'react-native';
import { PRIMARY_PALETTE, spacing, borderRadius, useThemeColors } from '../../constants/theme';
import type { ExtendedProviderStatus } from '../../types';

const PROVIDER_ICONS: Record<string, string> = {
  strava: '🚴',
  fitbit: '⌚',
  garmin: '⌚',
  whoop: '💪',
  coros: '🏃',
  terra: '🌍',
  synthetic: '🧪',
  synthetic_sleep: '😴',
};

interface ProviderModalProps {
  visible: boolean;
  providers: ExtendedProviderStatus[];
  connectingProvider: string | null;
  onClose: () => void;
  onSelectConnected: (provider: string) => void;
  onConnectProvider: (provider: string) => void;
  onConnectSciotte: (target: 'strava' | 'garmin') => void;
}

export function ProviderModal({
  visible,
  providers,
  connectingProvider,
  onClose,
  onSelectConnected,
  onConnectProvider,
  onConnectSciotte,
}: ProviderModalProps) {
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
          <Text className="text-lg font-semibold text-text-primary text-center mb-1">Connect a Provider</Text>
          <Text className="text-sm text-text-secondary text-center mb-6">
            To analyze your fitness data, please connect a provider first.
          </Text>

          {providers.map((provider) => {
            const icon = PROVIDER_ICONS[provider.provider] || '🔗';
            const isConnected = provider.connected;
            const requiresOAuth = provider.requires_oauth;
            const isSciotte = provider.provider.startsWith('sciotte');
            const isConnectable = isConnected || requiresOAuth || isSciotte;
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
                    {isConnecting ? `Connecting ${displayName}...` : isConnected ? displayName : `Connect ${displayName}`}
                  </Text>
                  {isConnected && (
                    <Text className="text-xs text-accent-primary">Connected ✓</Text>
                  )}
                </View>
              </TouchableOpacity>
            );
          })}

          <TouchableOpacity
            className="items-center p-4 mt-1"
            onPress={onClose}
          >
            <Text className="text-base text-text-tertiary">Cancel</Text>
          </TouchableOpacity>
        </View>
      </TouchableOpacity>
    </Modal>
  );
}
