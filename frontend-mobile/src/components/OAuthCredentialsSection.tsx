// ABOUTME: OAuth credentials management section for Settings screen
// ABOUTME: Allows users to register custom OAuth app credentials for providers

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  Alert,
  Modal,
  ActivityIndicator,
  FlatList,
} from 'react-native';
import { TouchableOpacity, GestureHandlerRootView } from 'react-native-gesture-handler';
import { PRIMARY_PALETTE, PROVIDER_COLORS } from '../constants/theme';
import { Card, Button, Input } from './ui';
import { userApi } from '../services/api';
import type { OAuthApp, OAuthProvider } from '../types';
import { useTranslation } from '@pierre/i18n';

const PROVIDERS: OAuthProvider[] = [
  { id: 'whoop', name: 'WHOOP', color: PROVIDER_COLORS.whoop },
];

const DEFAULT_REDIRECT_URI = 'https://pierre.fit/api/oauth/callback';

type ModalView = 'form' | 'providerPicker';

export function OAuthCredentialsSection() {
  const { t } = useTranslation();
  const [oauthApps, setOauthApps] = useState<OAuthApp[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [showAddModal, setShowAddModal] = useState(false);
  const [modalView, setModalView] = useState<ModalView>('form');

  // Form state
  const [selectedProvider, setSelectedProvider] = useState<OAuthProvider | null>(null);
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

  const handleSelectProvider = (provider: OAuthProvider) => {
    setSelectedProvider(provider);
    setModalView('form');
  };

  const getAvailableProviders = (): OAuthProvider[] => {
    const configuredIds = oauthApps.map(app => app.provider.toLowerCase());
    return PROVIDERS.filter(p => !configuredIds.includes(p.id.toLowerCase()));
  };

  const handleSave = async () => {
    if (!selectedProvider) {
      Alert.alert(t('common.error'), 'Please select a provider');
      return;
    }
    if (!clientId.trim()) {
      Alert.alert(t('common.error'), 'Please enter a Client ID');
      return;
    }
    if (!clientSecret.trim()) {
      Alert.alert(t('common.error'), 'Please enter a Client Secret');
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
      const message = error instanceof Error ? error.message : t('app.failedToSaveCredentials');
      Alert.alert(t('common.error'), message);
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = (provider: string, providerName: string) => {
    Alert.alert(
      'Remove Credentials',
      `Are you sure you want to remove ${providerName} credentials? You'll need to re-enter them to use a custom OAuth app.`,
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

  const getProviderInfo = (providerId: string): OAuthProvider => {
    return PROVIDERS.find(p => p.id.toLowerCase() === providerId.toLowerCase()) || {
      id: providerId,
      name: providerId.charAt(0).toUpperCase() + providerId.slice(1),
      color: PRIMARY_PALETTE[500],
    };
  };

  const maskClientId = (clientId: string): string => {
    if (clientId.length <= 8) return clientId;
    return `${clientId.substring(0, 8)}...`;
  };

  const availableProviders = getAvailableProviders();

  return (
    <View className="mt-3">
      <View className="flex-row justify-between items-center mb-1">
        <Text className="text-lg font-semibold text-text-primary">{t('app.oauthCredentials')}</Text>
        {availableProviders.length > 0 && (
          <TouchableOpacity
            className="px-3 py-2 min-h-[44px] justify-center"
            onPress={() => setShowAddModal(true)}
            hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
            activeOpacity={0.7}
          >
            <Text className="text-sm font-semibold text-primary-500">+ Add</Text>
          </TouchableOpacity>
        )}
      </View>

      <Text className="text-sm text-text-secondary mb-3">
        {t('app.oauthCredsBlurb')}
      </Text>

      <Card className="mb-3">
        {isLoading ? (
          <ActivityIndicator size="small" color={PRIMARY_PALETTE[500]} />
        ) : oauthApps.length === 0 ? (
          <Text className="text-sm text-text-secondary text-center py-3">
            {t('app.noOauthCreds')}
          </Text>
        ) : (
          oauthApps.map((app, index) => {
            const providerInfo = getProviderInfo(app.provider);
            return (
              <View
                key={app.provider}
                className={`py-2 ${index > 0 ? 'border-t border-border-subtle' : ''}`}
              >
                <View className="flex-row items-center mb-1">
                  <View
                    className="w-10 h-10 rounded-lg items-center justify-center mr-3"
                    style={{ backgroundColor: providerInfo.color }}
                  >
                    <Text className="text-lg font-bold text-text-primary">
                      {providerInfo.name.charAt(0).toUpperCase()}
                    </Text>
                  </View>
                  <View className="flex-1">
                    <View className="flex-row items-center mb-0.5">
                      <Text className="text-base font-semibold text-text-primary mr-2">
                        {providerInfo.name}
                      </Text>
                      <View className="bg-success/20 px-1 py-0.5 rounded">
                        <Text className="text-xs text-success font-medium">{t('app.configured')}</Text>
                      </View>
                    </View>
                    <Text className="text-sm text-text-tertiary font-mono">
                      {t('app.clientIdColon')} {maskClientId(app.client_id)}
                    </Text>
                  </View>
                </View>
                <TouchableOpacity onPress={() => handleDelete(app.provider, providerInfo.name)}>
                  <Text className="text-sm text-error font-medium">{t('app.remove')}</Text>
                </TouchableOpacity>
              </View>
            );
          })
        )}
      </Card>

      {/* Add Credentials Modal - single modal with view switching */}
      <Modal
        visible={showAddModal}
        animationType="slide"
        transparent
        onRequestClose={handleCloseModal}
      >
        <GestureHandlerRootView className="flex-1">
          <View className="flex-1 bg-black/70 justify-center px-4">
            {modalView === 'form' ? (
              <View className="bg-background-secondary rounded-xl p-4 max-h-[80%]">
                <Text className="text-xl font-semibold text-text-primary mb-4 text-center">
                  {t('app.addOauthCredentials')}
                </Text>

                {/* Provider Picker */}
                <Text className="text-sm font-medium text-text-secondary mb-1">{t('app.provider')}</Text>
                <TouchableOpacity
                  className="flex-row items-center justify-between bg-background-tertiary rounded-lg p-3 mb-3 border border-border-subtle"
                  onPress={() => setModalView('providerPicker')}
                  hitSlop={{ top: 5, bottom: 5, left: 5, right: 5 }}
                  activeOpacity={0.7}
                >
                  {selectedProvider ? (
                    <View className="flex-row items-center">
                      <View
                        className="w-6 h-6 rounded items-center justify-center mr-2"
                        style={{ backgroundColor: selectedProvider.color }}
                      >
                        <Text className="text-sm font-bold text-text-primary">
                          {selectedProvider.name.charAt(0)}
                        </Text>
                      </View>
                      <Text className="text-base text-text-primary">{selectedProvider.name}</Text>
                    </View>
                  ) : (
                    <Text className="text-base text-text-tertiary">{t('app.selectProviderPlaceholder')}</Text>
                  )}
                  <Text className="text-lg text-text-tertiary">{'>'}</Text>
                </TouchableOpacity>

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
                  Redirect URI (use this in your OAuth app)
                </Text>
                <View className="bg-background-tertiary rounded-lg p-3 mb-3 border border-border-subtle">
                  <Text className="text-sm text-text-secondary font-mono" selectable>
                    {selectedProvider ? `${DEFAULT_REDIRECT_URI}/${selectedProvider.id}` : DEFAULT_REDIRECT_URI}
                  </Text>
                </View>

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
              <View className="bg-background-secondary rounded-xl p-4 max-h-[60%]">
                <Text className="text-xl font-semibold text-text-primary mb-4 text-center">
                  {t('app.selectProvider')}
                </Text>
                <FlatList
                  data={availableProviders}
                  keyExtractor={(item) => item.id}
                  renderItem={({ item }) => (
                    <TouchableOpacity
                      className="flex-row items-center py-3"
                      onPress={() => handleSelectProvider(item)}
                    >
                      <View
                        className="w-10 h-10 rounded-lg items-center justify-center mr-3"
                        style={{ backgroundColor: item.color }}
                      >
                        <Text className="text-lg font-bold text-text-primary">{item.name.charAt(0)}</Text>
                      </View>
                      <Text className="flex-1 text-base text-text-primary ml-2">{item.name}</Text>
                      {selectedProvider?.id === item.id && (
                        <Text className="text-lg text-primary-500">{'✓'}</Text>
                      )}
                    </TouchableOpacity>
                  )}
                  ItemSeparatorComponent={() => <View className="h-px bg-border-subtle" />}
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
        </GestureHandlerRootView>
      </Modal>
    </View>
  );
}
