// ABOUTME: Provider selection dialog for connecting fitness data providers
// ABOUTME: A centred Modal listing every provider with its brand glyph, its connection status and the OAuth flow it starts

import React from 'react';
import { View, Text, TouchableOpacity, Modal, ActivityIndicator } from 'react-native';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';
import { InitialsAvatar } from '../../components/ui';
import { providerGlyph } from '../../components/icons/BrandIcons';
import type { ExtendedProviderStatus } from '../../types';
import { useTranslation } from '@pierre/i18n';

/** The glyph slot's edge: every brand mark and the initials circle share it, so the names line up. */
const GLYPH_SIZE = 24;

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

/**
 * The brand mark for a provider id, or an initials circle when
 * `providerGlyph` knows no mark for it — a provider the server reports that
 * the glyph map has not caught up with still gets a face, in a slot colour
 * rather than a brand colour it does not have.
 */
function ProviderGlyph({ providerId, label }: { providerId: string; label: string }) {
  const Glyph = providerGlyph(providerId);
  if (Glyph) {
    return <Glyph size={GLYPH_SIZE} />;
  }
  return (
    <InitialsAvatar
      initials={initialsFor(label)}
      slot={avatarSlot({ id: providerId, agent_id: null, group_id: null })}
      size={GLYPH_SIZE}
    />
  );
}

/**
 * A dialog, not a sheet: it floats at the centre on the scrim, so it is the
 * one surface that carries `shadow-floating`. Each provider is a 52-tall row
 * under a faint hairline, the last one without.
 */
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
  return (
    <Modal
      visible={visible}
      animationType="fade"
      transparent
      onRequestClose={onClose}
    >
      <TouchableOpacity
        className="flex-1 bg-scrim/60 justify-center items-center"
        activeOpacity={1}
        onPress={onClose}
      >
        <View
          className="w-[300px] rounded-xl shadow-floating px-4 py-4"
          style={{ backgroundColor: colors.background.secondary }}
          onStartShouldSetResponder={() => true}
          testID="provider-modal"
        >
          <Text className="text-lg font-semibold text-text-primary text-center mb-1">{t('app.connectAProvider')}</Text>
          <Text className="text-sm text-text-secondary text-center mb-4">
            {t('app.connectFirstBlurb')}
          </Text>

          {providers.map((provider, index) => {
            const isConnected = provider.connected;
            const requiresOAuth = provider.requires_oauth;
            const isSciotte = provider.provider.startsWith('sciotte');
            const isIntervals = provider.provider === 'intervals_icu';
            const isConnectable = isConnected || requiresOAuth || isSciotte || isIntervals;
            const displayName = provider.display_name || provider.provider;
            const isConnecting = connectingProvider === provider.provider;
            const isOtherConnecting = connectingProvider !== null && !isConnecting;
            const isLast = index === providers.length - 1;

            return (
              <TouchableOpacity
                key={provider.provider}
                className={`flex-row items-center min-h-[52px] gap-3 ${isLast ? '' : 'border-b border-border-faint'}`}
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
                accessibilityRole="button"
                testID={`provider-row-${provider.provider}`}
              >
                <View className="items-center justify-center" style={{ width: GLYPH_SIZE, height: GLYPH_SIZE }}>
                  {isConnecting ? (
                    <ActivityIndicator size="small" color={colors.tokens.primary} />
                  ) : (
                    <ProviderGlyph providerId={provider.provider} label={displayName} />
                  )}
                </View>
                <Text
                  className={`flex-1 text-base ${isOtherConnecting ? 'text-text-tertiary' : 'text-text-primary'}`}
                  numberOfLines={1}
                >
                  {isConnecting
                    ? t('app.connectingProvider', { provider: displayName })
                    : isConnected
                      ? displayName
                      : t('app.connectProvider', { provider: displayName })}
                </Text>
                {isConnected ? (
                  <Text className="text-sm font-medium text-primary">{t('app.connectedCheck')}</Text>
                ) : null}
              </TouchableOpacity>
            );
          })}

          <TouchableOpacity
            className="items-center py-3 mt-2"
            onPress={onClose}
            accessibilityRole="button"
            testID="provider-modal-cancel"
          >
            <Text className="text-base text-text-tertiary">{t('common.cancel')}</Text>
          </TouchableOpacity>
        </View>
      </TouchableOpacity>
    </Modal>
  );
}
