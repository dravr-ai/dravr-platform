// ABOUTME: AI provider settings — bring your own key, validate it, or remove it
// ABOUTME: Mobile counterpart of the web Settings LLM tab, on the same shared endpoints

import React, { useCallback, useState } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter, useFocusEffect } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import type { LlmSettingsResponse } from '@pierre/api-client';
import { spacing, useThemeColors } from '../../constants/theme';
import { Input } from '../../components/ui';
import { userApi } from '../../services/api';
import { useTranslation } from '@pierre/i18n';

/**
 * Manage the athlete's own AI provider credentials.
 *
 * Mirrors the web Settings AI-provider tab: it shows which provider is active
 * and where its key comes from, lets the user supply their own, and lets them
 * remove it again. A key is validated against the provider before it is saved,
 * so a typo fails here rather than silently breaking every later coaching turn.
 */
export function LlmSettingsScreen() {
  const { t } = useTranslation();
  const router = useRouter();
  const colors = useThemeColors();

  const [settings, setSettings] = useState<LlmSettingsResponse | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const [apiKey, setApiKey] = useState('');
  const [isSaving, setIsSaving] = useState(false);

  const load = useCallback(async () => {
    try {
      setIsLoading(true);
      setError(null);
      setSettings(await userApi.getLlmSettings());
    } catch (err) {
      setError(err instanceof Error ? err.message : t('app.failedLoadAiSettings'));
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useFocusEffect(
    useCallback(() => {
      void load();
    }, [load]),
  );

  const handleSave = async () => {
    if (!selectedProvider || apiKey.trim().length === 0) {
      return;
    }
    setIsSaving(true);
    try {
      // Validate before saving: an invalid key stored now would surface as a
      // failed coaching turn later, far from the screen that caused it.
      const check = await userApi.validateLlmCredentials({
        provider: selectedProvider,
        api_key: apiKey.trim(),
      });
      if (!check.valid) {
        Alert.alert(t('app.keyRejected'), check.error ?? `${selectedProvider} did not accept that key.`);
        return;
      }
      await userApi.saveLlmCredentials({ provider: selectedProvider, api_key: apiKey.trim() });
      setApiKey('');
      setSelectedProvider(null);
      await load();
      Alert.alert(t('app.providerSaved'), t('app.keyValidatedSaved'));
    } catch (err) {
      Alert.alert(t('app.couldNotSaveKey'), err instanceof Error ? err.message : t('app.unknownError'));
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = (provider: string) => {
    Alert.alert(
      `Remove your ${provider} key?`,
      t('app.coachingFallsBack'),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.remove'),
          style: 'destructive',
          onPress: () => {
            void (async () => {
              try {
                await userApi.deleteLlmCredentials(provider);
                await load();
              } catch (err) {
                Alert.alert(t('app.couldNotRemoveKey'), err instanceof Error ? err.message : t('app.unknownError'));
              }
            })();
          },
        },
      ],
    );
  };

  const cardStyle: ViewStyle = {
    backgroundColor: colors.background.tertiary,
    borderWidth: 1,
    borderColor: colors.border.default,
    borderRadius: 16,
    overflow: 'hidden',
  };

  const rowStyle: ViewStyle = {
    flexDirection: 'row',
    alignItems: 'center',
    paddingVertical: 14,
    paddingHorizontal: 16,
  };

  const providers = settings?.providers ?? [];
  const ownKeyProviders = new Set((settings?.user_credentials ?? []).map((c) => c.provider));

  return (
    <SafeAreaView style={{ flex: 1, backgroundColor: colors.background.primary }} edges={['top']} testID="llm-settings-screen">
      <View style={{ flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.md, paddingVertical: spacing.sm }}>
        <TouchableOpacity onPress={() => router.back()} testID="back-button" style={{ padding: 8, marginRight: 8 }}>
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text style={{ fontSize: 20, fontWeight: '600', color: colors.text.primary }}>{t('app.aiProvider')}</Text>
      </View>

      {isLoading ? (
        <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center' }}>
          <ActivityIndicator size="large" color={colors.pierre.violet} />
        </View>
      ) : error ? (
        <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center', padding: spacing.lg, gap: 12 }}>
          <Text style={{ color: colors.text.secondary, textAlign: 'center' }} testID="llm-error">{error}</Text>
          <TouchableOpacity onPress={() => { void load(); }} testID="llm-retry" style={{ paddingHorizontal: 20, paddingVertical: 12, borderRadius: 12, backgroundColor: colors.background.tertiary }}>
            <Text style={{ color: colors.text.primary, fontWeight: '600' }}>{t('common.retry')}</Text>
          </TouchableOpacity>
        </View>
      ) : (
        <ScrollView contentContainerStyle={{ padding: spacing.md, gap: spacing.lg }}>
          <View>
            <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>{t('app.providers')}</Text>
            <View style={cardStyle}>
              {providers.map((provider, index) => (
                <View
                  key={provider.name}
                  style={[
                    rowStyle,
                    index < providers.length - 1
                      ? { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }
                      : {},
                  ]}
                  testID={`llm-provider-${provider.name}`}
                >
                  <View style={{ flex: 1 }}>
                    <Text style={{ fontSize: 16, color: colors.text.primary }}>{provider.display_name}</Text>
                    <Text style={{ fontSize: 14, color: colors.text.tertiary }}>
                      {provider.has_credentials
                        ? t('app.keyFromSource', { source: provider.credential_source ?? 'system' })
                        : t('app.noKeyConfigured')}
                      {provider.is_active ? ' · Active' : ''}
                    </Text>
                  </View>
                  {ownKeyProviders.has(provider.name) ? (
                    <TouchableOpacity
                      onPress={() => handleDelete(provider.name)}
                      testID={`llm-remove-${provider.name}`}
                      style={{ paddingHorizontal: 12, paddingVertical: 8 }}
                    >
                      <Text style={{ color: colors.pierre.red, fontWeight: '600' }}>{t('app.remove')}</Text>
                    </TouchableOpacity>
                  ) : (
                    <TouchableOpacity
                      onPress={() => setSelectedProvider(provider.name)}
                      testID={`llm-select-${provider.name}`}
                      style={{ paddingHorizontal: 12, paddingVertical: 8 }}
                    >
                      <Text style={{ color: colors.pierre.violet, fontWeight: '600' }}>{t('app.addKey')}</Text>
                    </TouchableOpacity>
                  )}
                </View>
              ))}
            </View>
          </View>

          {selectedProvider && (
            <View testID="llm-key-form">
              <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>
                {selectedProvider} key
              </Text>
              <View style={{ ...cardStyle, padding: 16, gap: 12 }}>
                <Input
                  value={apiKey}
                  onChangeText={setApiKey}
                  placeholder={t('app.pasteApiKey')}
                  autoCapitalize="none"
                  secureTextEntry
                  testID="llm-api-key-input"
                />
                <TouchableOpacity
                  onPress={() => { void handleSave(); }}
                  disabled={apiKey.trim().length === 0 || isSaving}
                  testID="llm-save-button"
                  style={{
                    backgroundColor: apiKey.trim().length > 0 && !isSaving ? colors.tokens.primary : colors.background.secondary,
                    borderRadius: 12,
                    paddingVertical: 14,
                    alignItems: 'center',
                  }}
                >
                  {isSaving ? (
                    <ActivityIndicator color={colors.tokens.onPrimary} />
                  ) : (
                    <Text style={{ fontWeight: '600', color: apiKey.trim().length > 0 ? colors.tokens.onPrimary : colors.text.tertiary }}>
                      {t('app.validateAndSave')}
                    </Text>
                  )}
                </TouchableOpacity>
              </View>
            </View>
          )}
        </ScrollView>
      )}
    </SafeAreaView>
  );
}
