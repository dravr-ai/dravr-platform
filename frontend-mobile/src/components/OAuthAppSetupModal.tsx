// ABOUTME: Provider-aware BYO OAuth app sheet — captures client_id/secret inline and shows the redirect URI to register
// ABOUTME: Mobile mirror of frontend/src/components/OAuthAppSetupModal.tsx; opens in-place so first-run users never leave the screen

import React, { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, ScrollView, Text, View } from 'react-native';
import { Button, Input, Sheet } from './ui';
import { userApi } from '../services/api';
import { useThemeColors } from '../constants/theme';
import type { OAuthApp } from '../types';
import { Trans, useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { openExternal } from '../utils/openExternal';

interface OAuthAppSetupModalProps {
  visible: boolean;
  onClose: () => void;
  /** Fires after the OAuth app is saved successfully — parent kicks off the OAuth dance. */
  onSaved: () => void;
  /** Provider id matching the server enum, e.g. `whoop`, `strava`. */
  provider: string;
  /** Human-readable name shown in the sheet title and copy. */
  displayName: string;
  /** Developer-portal URL where the user creates their OAuth app. */
  devPortalUrl: string;
  /**
   * The server's callback for this provider (`oauth_callback_url` on its
   * status), the redirect URI the user registers on the developer portal.
   */
  callbackUrl?: string;
}

/** Where the callback lives when neither the status nor a saved app names it. */
const FALLBACK_REDIRECT_HOST = 'https://app.dravr.ai';

export function OAuthAppSetupModal({
  visible,
  onClose,
  onSaved,
  provider,
  displayName,
  devPortalUrl,
  callbackUrl,
}: OAuthAppSetupModalProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();

  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
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
      setClientId(existing?.client_id ?? '');
      setClientSecret('');
    } catch (err) {
      // Non-fatal: a load failure just means the user fills the form fresh.
      console.warn('OAuthAppSetupModal: failed to hydrate existing app', err);
      setExistingApp(null);
    } finally {
      setIsLoadingExisting(false);
    }
  }, [provider]);

  useEffect(() => {
    if (!visible) return;
    setError(null);
    void hydrate();
  }, [visible, hydrate]);

  // A device that cannot open the portal is told which one to open by hand.
  const handleOpenDevPortal = () => {
    void openExternal(devPortalUrl, t, {
      title: t('app.unableOpenBrowser'),
      message: t('app.openPortalManually', { url: devPortalUrl, provider: displayName }),
    });
  };

  // The server authorizes every app at its own callback; a saved app and the
  // provider status both name it.
  const redirectUri =
    existingApp?.redirect_uri ??
    callbackUrl ??
    `${FALLBACK_REDIRECT_HOST}/api/oauth/callback/${provider}`;

  const handleSubmit = async () => {
    const trimmedId = clientId.trim();
    const trimmedSecret = clientSecret.trim();

    if (!trimmedId || !trimmedSecret) {
      setError(t('settingsErr.credentialsRequired'));
      return;
    }

    try {
      setIsSaving(true);
      setError(null);
      await userApi.registerOAuthApp({
        provider,
        client_id: trimmedId,
        client_secret: trimmedSecret,
      });
      onSaved();
    } catch (err) {
      // The fallback sentence names the provider, so the translator handed to
      // the classifier fills it in; every other key ignores the extra value.
      setError(
        describeApiError(err, {
          t: (key, params) => t(key, { provider: displayName, ...params }),
          fallbackKey: 'app.failedSaveOauthApp',
        }),
      );
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
    <Sheet
      visible={visible}
      onClose={onClose}
      maxHeight="max-h-[92%]"
      testID="oauth-app-setup-sheet"
      backdropTestID="oauth-app-setup-backdrop"
    >
      <Text className="text-xl font-semibold text-text-primary mb-2">
        {t('app.setUpOauthApp', { provider: displayName })}
      </Text>

      <ScrollView keyboardShouldPersistTaps="handled" showsVerticalScrollIndicator={false}>
        <Text className="text-sm text-text-secondary mb-3 leading-5">
          <Trans
            i18nKey="app.oauthAppSetupIntro"
            values={{ provider: displayName, host: devPortalHost }}
            components={{
              portal: <Text className="text-primary underline" onPress={handleOpenDevPortal} />,
            }}
          />
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
          editable={false}
          selectTextOnFocus
          autoCapitalize="none"
          autoCorrect={false}
        />
        <Text className="text-xs text-text-tertiary mb-3">
          {t('app.addExactUri', { provider: displayName })}
        </Text>

        {error ? (
          <Text accessibilityLiveRegion="polite" className="text-sm text-error mb-3">
            {error}
          </Text>
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
    </Sheet>
  );
}
