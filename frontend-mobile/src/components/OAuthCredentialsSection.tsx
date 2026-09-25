// ABOUTME: OAuth credentials section — the athlete's own OAuth app credentials per provider, as a Section with an Add action
// ABOUTME: The add form is a centred dialog on the secondary ground, radius 12 and flat; the provider picker is its second view

import React, { useState, useEffect, useCallback } from 'react';
import { View, Text, Alert, Modal, ActivityIndicator, FlatList, Pressable } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useThemeColors } from '../constants/theme';
import { Button, Input, Section } from './ui';
import { ProviderGlyph } from './ProviderGlyph';
import { userApi } from '../services/api';
import type { OAuthApp } from '../types';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';

/** A provider an athlete can register their own OAuth app for. */
interface ByoProvider {
  id: string;
  name: string;
}

// After the 2026-Q2 provider cleanup, BYO-OAuth-app is WHOOP-only.
const PROVIDERS: ByoProvider[] = [{ id: 'whoop', name: 'WHOOP' }];

const DEFAULT_REDIRECT_URI = 'https://pierre.fit/api/oauth/callback';

type ModalView = 'form' | 'providerPicker';

export function OAuthCredentialsSection() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const [oauthApps, setOauthApps] = useState<OAuthApp[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [showAddModal, setShowAddModal] = useState(false);
  const [modalView, setModalView] = useState<ModalView>('form');

  // Form state
  const [selectedProvider, setSelectedProvider] = useState<ByoProvider | null>(null);
  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
  const [isSaving, setIsSaving] = useState(false);

  const loadOAuthApps = useCallback(async () => {
    try {
      setIsLoading(true);
      const response = await userApi.getOAuthApps();
      setOauthApps(response.apps || []);
    } catch (error) {
      console.error('Failed to load OAuth apps:', error);
      setOauthApps([]);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadOAuthApps();
  }, [loadOAuthApps]);

  const resetForm = () => {
    setSelectedProvider(null);
    setClientId('');
    setClientSecret('');
    setModalView('form');
  };

  const handleCloseModal = () => {
    setShowAddModal(false);
    resetForm();
  };

  const handleSelectProvider = (provider: ByoProvider) => {
    setSelectedProvider(provider);
    setModalView('form');
  };

  const getAvailableProviders = (): ByoProvider[] => {
    const configuredIds = oauthApps.map(app => app.provider.toLowerCase());
    return PROVIDERS.filter(p => !configuredIds.includes(p.id.toLowerCase()));
  };

  const handleSave = async () => {
    if (!selectedProvider) {
      Alert.alert(t('common.error'), t('app.pleaseSelectProvider'));
      return;
    }
    if (!clientId.trim()) {
      Alert.alert(t('common.error'), t('app.pleaseEnterClientId'));
      return;
    }
    if (!clientSecret.trim()) {
      Alert.alert(t('common.error'), t('app.pleaseEnterClientSecret'));
      return;
    }

    try {
      setIsSaving(true);
      await userApi.registerOAuthApp({
        provider: selectedProvider.id,
        client_id: clientId.trim(),
        client_secret: clientSecret.trim(),
        redirect_uri: `${DEFAULT_REDIRECT_URI}/${selectedProvider.id}`,
      });
      Alert.alert(t('common.success'), t('app.credentialsSavedFor', { provider: selectedProvider.name }));
      handleCloseModal();
      await loadOAuthApps();
    } catch (error) {
      const message = describeApiError(error, { t, fallbackKey: 'app.failedToSaveCredentials' });
      Alert.alert(t('common.error'), message);
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = (provider: string, providerName: string) => {
    Alert.alert(
      t('app.removeCredentials'),
      t('app.confirmRemoveProviderCreds', { provider: providerName }),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.remove'),
          style: 'destructive',
          onPress: async () => {
            try {
              await userApi.deleteOAuthApp(provider);
              await loadOAuthApps();
            } catch {
              Alert.alert(t('common.error'), t('app.failedRemoveCredentials'));
            }
          },
        },
      ]
    );
  };

  const getProviderInfo = (providerId: string): ByoProvider => {
    return PROVIDERS.find(p => p.id.toLowerCase() === providerId.toLowerCase()) || {
      id: providerId,
      name: providerId.charAt(0).toUpperCase() + providerId.slice(1),
    };
  };

  const maskClientId = (clientId: string): string => {
    if (clientId.length <= 8) return clientId;
    return `${clientId.substring(0, 8)}...`;
  };

  const availableProviders = getAvailableProviders();

  return (
    <Section
      title={t('app.oauthCredentials')}
      description={t('app.oauthCredsBlurb')}
      testID="oauth-credentials-section"
      actions={
        availableProviders.length > 0 ? (
          <Text
            className="text-sm font-medium text-primary py-2"
            onPress={() => setShowAddModal(true)}
            accessibilityRole="button"
            testID="oauth-credentials-add"
          >
            {t('common.add')}
          </Text>
        ) : undefined
      }
    >
      {/* The Section's content is full-bleed and these are plain lines and
          custom rows, not settings `Row`s, so each pays the header's inset
          itself to share its left edge. */}
      {isLoading ? (
        <ActivityIndicator size="small" color={colors.tokens.primary} />
      ) : oauthApps.length === 0 ? (
        <Text className="text-sm text-text-secondary px-4 py-3">{t('app.noOauthCreds')}</Text>
      ) : (
        oauthApps.map((app, index) => {
          const providerInfo = getProviderInfo(app.provider);
          return (
            <View
              key={app.provider}
              className={`flex-row items-center min-h-[52px] px-4 py-2 ${index > 0 ? 'border-t border-border-faint' : ''}`}
            >
              <View className="w-6 items-center mr-3.5">
                <ProviderGlyph providerId={app.provider} label={providerInfo.name} />
              </View>
              <View className="flex-1 min-w-0">
                <Text className="text-base text-text-primary">{providerInfo.name}</Text>
                <Text className="text-sm text-text-tertiary font-mono" numberOfLines={1}>
                  {t('app.clientIdColon')} {maskClientId(app.client_id)}
                </Text>
              </View>
              <View className="flex-row items-center gap-2 ml-3">
                <Text className="text-sm font-medium text-text-secondary">{t('app.configured')}</Text>
                <Text
                  className="text-md font-medium text-error"
                  onPress={() => handleDelete(app.provider, providerInfo.name)}
                  accessibilityRole="button"
                >
                  {t('app.remove')}
                </Text>
              </View>
            </View>
          );
        })
      )}

      {/* The add form: a centred dialog on the secondary ground, flat, with the
          provider picker as its second view. A dialog rather than a sheet because
          it already opens from inside one. */}
      <Modal
        visible={showAddModal}
        animationType="fade"
        transparent
        onRequestClose={handleCloseModal}
      >
        <View className="flex-1 bg-scrim/60 justify-center px-4">
          {modalView === 'form' ? (
            <View className="bg-background-secondary rounded-xl p-4 max-h-[80%]" testID="oauth-credentials-form">
              <Text className="text-xl font-semibold text-text-primary mb-4 text-center">
                {t('app.addOauthCredentials')}
              </Text>

              {/* Provider Picker */}
              <Text className="text-sm font-medium text-text-secondary mb-1">{t('app.provider')}</Text>
              <Pressable
                className="flex-row items-center justify-between py-3 mb-3 border-b border-border"
                onPress={() => setModalView('providerPicker')}
                accessibilityRole="button"
                testID="oauth-credentials-provider-picker"
              >
                {selectedProvider ? (
                  <View className="flex-row items-center gap-2">
                    <ProviderGlyph providerId={selectedProvider.id} label={selectedProvider.name} size={20} />
                    <Text className="text-base text-text-primary">{selectedProvider.name}</Text>
                  </View>
                ) : (
                  <Text className="text-base text-text-tertiary">{t('app.selectProviderPlaceholder')}</Text>
                )}
                <Text className="text-lg text-text-tertiary">{'>'}</Text>
              </Pressable>

              <Input
                label={t('app.clientId')}
                placeholder={t('app.enterOauthClientId')}
                value={clientId}
                onChangeText={setClientId}
                autoCapitalize="none"
                autoCorrect={false}
              />

              <Input
                label={t('app.clientSecret')}
                placeholder={t('app.enterOauthClientSecret')}
                value={clientSecret}
                onChangeText={setClientSecret}
                secureTextEntry
                showPasswordToggle
                autoCapitalize="none"
                autoCorrect={false}
              />

              {/* Redirect URI - read-only, shown for user to configure in OAuth app */}
              <Text className="text-sm font-medium text-text-secondary mb-1">
                {t('app.redirectUriHint')}
              </Text>
              <Text className="text-sm text-text-secondary font-mono py-3 mb-3 border-b border-border" selectable>
                {selectedProvider ? `${DEFAULT_REDIRECT_URI}/${selectedProvider.id}` : DEFAULT_REDIRECT_URI}
              </Text>

              <View className="flex-row gap-3 mt-3">
                <Button
                  title={t('common.cancel')}
                  onPress={handleCloseModal}
                  variant="secondary"
                  style={{ flex: 1 }}
                />
                <Button
                  title={t('common.save')}
                  onPress={handleSave}
                  loading={isSaving}
                  style={{ flex: 1 }}
                />
              </View>
            </View>
          ) : (
            <View className="bg-background-secondary rounded-xl p-4 max-h-[60%]" testID="oauth-credentials-picker">
              <Text className="text-xl font-semibold text-text-primary mb-4 text-center">
                {t('app.selectProvider')}
              </Text>
              <FlatList
                data={availableProviders}
                keyExtractor={(item) => item.id}
                renderItem={({ item }) => (
                  <Pressable
                    className="flex-row items-center min-h-[52px] py-2"
                    onPress={() => handleSelectProvider(item)}
                    accessibilityRole="button"
                  >
                    <View className="w-6 items-center mr-3.5">
                      <ProviderGlyph providerId={item.id} label={item.name} />
                    </View>
                    <Text className="flex-1 text-base text-text-primary">{item.name}</Text>
                    {selectedProvider?.id === item.id && (
                      <Feather name="check" size={18} color={colors.tokens.primary} />
                    )}
                  </Pressable>
                )}
                ItemSeparatorComponent={() => <View className="h-px bg-border-faint" />}
              />
              <Button
                title={t('common.back')}
                onPress={() => setModalView('form')}
                variant="secondary"
                fullWidth
                style={{ marginTop: 12 }}
              />
            </View>
          )}
        </View>
      </Modal>
    </Section>
  );
}
