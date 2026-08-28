// ABOUTME: Provider-aware BYO OAuth app modal — captures client_id/secret/redirect URI inline during the connect flow
// ABOUTME: Mobile mirror of frontend/src/components/OAuthAppSetupModal.tsx; opens in-place so first-run users never leave the screen

import React, { useCallback, useEffect, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  Linking,
  Modal,
  ScrollView,
  Text,
  View,
} from 'react-native';
import { TouchableOpacity } from 'react-native-gesture-handler';
import { Feather } from '@expo/vector-icons';
import { Button, Input } from './ui';
import { userApi } from '../services/api';
import { useThemeColors } from '../constants/theme';
import type { OAuthApp } from '../types';
import { useTranslation } from '@pierre/i18n';

interface OAuthAppSetupModalProps {
  visible: boolean;
  onClose: () => void;
  /** Fires after the OAuth app is saved successfully — parent kicks off the OAuth dance. */
  onSaved: () => void;
  /** Provider id matching the server enum, e.g. `whoop`, `strava`. */
  provider: string;
  /** Human-readable name shown in the modal title and copy. */
  displayName: string;
  /** Developer-portal URL where the user creates their OAuth app. */
  devPortalUrl: string;
  /**
   * Redirect URI to register on the developer portal. Defaults to the
   * production callback path; ConnectionsScreen overrides this with the
   * mobile deep-link scheme when needed.
   */
  defaultRedirectUri?: string;
}

const FALLBACK_REDIRECT_HOST = 'https://app.dravr.ai';

export function OAuthAppSetupModal({
  visible,
  onClose,
  onSaved,
  provider,
  displayName,
  devPortalUrl,
  defaultRedirectUri,
}: OAuthAppSetupModalProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const resolvedRedirectDefault =
    defaultRedirectUri ?? `${FALLBACK_REDIRECT_HOST}/api/oauth/callback/${provider}`;

  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
  const [redirectUri, setRedirectUri] = useState(resolvedRedirectDefault);
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [existingApp, setExistingApp] = useState<OAuthApp | null>(null);
  const [isLoadingExisting, setIsLoadingExisting] = useState(false);

  // Pre-populate from any previously saved app for this provider so a returning
  // user only has to re-enter the secret (which the API never returns).
  const hydrate = useCallback(async () => {
    try {
      setIsLoadingExisting(true);
      const response = await userApi.getOAuthApps();
      const existing =
        response.apps?.find((app) => app.provider.toLowerCase() === provider.toLowerCase()) ??
        null;
      setExistingApp(existing);
      if (existing) {
        setClientId(existing.client_id ?? '');
        setClientSecret('');
        setRedirectUri(existing.redirect_uri ?? resolvedRedirectDefault);
      } else {
        setClientId('');
        setClientSecret('');
        setRedirectUri(resolvedRedirectDefault);
      }
    } catch (err) {
      // Non-fatal: a load failure just means the user fills the form fresh.
      console.warn('OAuthAppSetupModal: failed to hydrate existing app', err);
      setExistingApp(null);
    } finally {
      setIsLoadingExisting(false);
    }
  }, [provider, resolvedRedirectDefault]);

  useEffect(() => {
    if (!visible) return;
    setError(null);
    void hydrate();
  }, [visible, hydrate]);

  const handleOpenDevPortal = async () => {
    try {
      await Linking.openURL(devPortalUrl);
    } catch {
      Alert.alert(
        t('app.unableOpenBrowser'),
        t('app.openPortalManually', { url: devPortalUrl, provider: displayName }),
      );
    }
  };

  const handleSubmit = async () => {
    const trimmedId = clientId.trim();
    const trimmedSecret = clientSecret.trim();
    const trimmedRedirect = redirectUri.trim();

    if (!trimmedId || !trimmedSecret || !trimmedRedirect) {
      setError(t('app.oauthAllFieldsRequired'));
      return;
    }

    try {
      setIsSaving(true);
      setError(null);
      await userApi.registerOAuthApp({
        provider,
        client_id: trimmedId,
        client_secret: trimmedSecret,
        redirect_uri: trimmedRedirect,
      });
      onSaved();
    } catch (err) {
      const message =
        err instanceof Error
          ? err.message
          : t('app.failedSaveOauthApp', { provider: displayName });
      setError(message);
    } finally {
      setIsSaving(false);
    }
  };

  const devPortalHost = (() => {
    try {
      return new URL(devPortalUrl).host;
    } catch {
      return devPortalUrl;
    }
  })();

  return (
    <Modal
      visible={visible}
      animationType="slide"
      transparent
      onRequestClose={onClose}
    >
      <View className="flex-1 bg-black/70 justify-end">
        <View
          className="bg-background-primary rounded-t-3xl pt-3 pb-10 px-4 max-h-[92%]"
          accessibilityViewIsModal
        >
          <View className="items-center mb-3">
            <View className="w-10 h-1 rounded-full bg-border-default" />
          </View>

          <View className="flex-row items-center justify-between mb-2">
            <Text className="text-xl font-semibold text-text-primary flex-1">
              {t('app.setUpOauthApp', { provider: displayName })}
            </Text>
            <TouchableOpacity
              onPress={onClose}
              hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
              accessibilityRole="button"
              accessibilityLabel={t('common.close')}
            >
              <Feather name="x" size={22} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>

          <ScrollView
            keyboardShouldPersistTaps="handled"
            showsVerticalScrollIndicator={false}
          >
            <Text className="text-sm text-text-secondary mb-3 leading-5">
              {displayName} requires each user to register their own developer
              app. Create one at{' '}
              <Text
                className="text-primary-500 underline"
                onPress={handleOpenDevPortal}
              >
                {devPortalHost}
              </Text>{' '}
              and use the redirect URI shown below.
            </Text>

            {isLoadingExisting ? (
              <View className="py-6 items-center">
                <ActivityIndicator size="small" color={colors.text.secondary} />
              </View>
            ) : null}

            <Input
              label={t('app.clientId')}
              placeholder={t('app.pasteProviderClientId', { provider: displayName })}
              value={clientId}
              onChangeText={setClientId}
              autoCapitalize="none"
              autoCorrect={false}
            />

            <Input
              label={t('app.clientSecret')}
              placeholder={
                existingApp
                  ? t('app.reenterProviderClientSecret', { provider: displayName })
                  : t('app.pasteProviderClientSecret', { provider: displayName })
              }
              value={clientSecret}
              onChangeText={setClientSecret}
              secureTextEntry
              showPasswordToggle
              autoCapitalize="none"
              autoCorrect={false}
            />

            <Input
              label={t('app.redirectUri')}
              value={redirectUri}
              onChangeText={setRedirectUri}
              autoCapitalize="none"
              autoCorrect={false}
            />
            <Text className="text-xs text-text-tertiary mb-3">
              {t('app.addExactUri', { provider: displayName })}
            </Text>

            {error ? (
              <View
                accessibilityLiveRegion="polite"
                className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 mb-3"
              >
                <Text className="text-sm text-error">{error}</Text>
              </View>
            ) : null}

            <View className="flex-row gap-3 mt-2">
              <Button
                title={t('common.cancel')}
                onPress={onClose}
                variant="secondary"
                style={{ flex: 1 }}
              />
              <Button
                title={isSaving ? t('app.saving') : t('app.saveAndConnect')}
                onPress={handleSubmit}
                loading={isSaving}
                style={{ flex: 1 }}
              />
            </View>
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}
