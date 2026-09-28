// ABOUTME: API keys pane — the athlete's own API keys as 52 rows (name, prefix and 30-day use) with an ink Revoke
// ABOUTME: Creating one happens in the bottom Sheet, which shows the new key this once; @pierre/api-client's apiKeys domain

import React, { useCallback, useEffect, useState } from 'react';
import { View, Text, ActivityIndicator, Alert } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { API_KEY_USAGE_WINDOW_DAYS } from '@pierre/shared-constants';
import type { ApiKeyInfo } from '@pierre/shared-types';
import { spacing, useThemeColors } from '../../constants/theme';
import { Button, EmptyState, Input, PaneScrollView, Row, Section, Sheet } from '../../components/ui';
import { apiKeysApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';

const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * Manage the API keys the athlete's own scripts call Dravr with.
 *
 * A key is a bearer credential, so the pane that mints one can also take it
 * back; a revoked key leaves the list. The full key is shown once, in the
 * sheet that created it, and never again.
 */
export function ApiKeysScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { isAuthenticated } = useAuth();

  const [keys, setKeys] = useState<ApiKeyInfo[]>([]);
  const [requests, setRequests] = useState<Record<string, number>>({});
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [revokingKeyId, setRevokingKeyId] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [name, setName] = useState('');
  const [isCreating, setIsCreating] = useState(false);
  const [createdKey, setCreatedKey] = useState<string | null>(null);

  const loadKeys = useCallback(async () => {
    try {
      setLoadError(null);
      const response = await apiKeysApi.list();
      const active = response.api_keys.filter((key) => key.is_active);
      setKeys(active);
      const end = new Date();
      const start = new Date(end.getTime() - API_KEY_USAGE_WINDOW_DAYS * DAY_MS);
      const usages = await Promise.allSettled(
        active.map((key) => apiKeysApi.usage(key.id, start.toISOString(), end.toISOString())),
      );
      const counts: Record<string, number> = {};
      usages.forEach((result, index) => {
        if (result.status === 'fulfilled') counts[active[index].id] = result.value.stats.total_requests;
      });
      setRequests(counts);
    } catch (err) {
      setLoadError(describeApiError(err, { t, fallbackKey: 'apiKeys.loadFailed' }));
      setKeys([]);
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    if (isAuthenticated) {
      void loadKeys();
    }
  }, [isAuthenticated, loadKeys]);

  const handleCreate = async () => {
    if (!name.trim()) return;
    try {
      setIsCreating(true);
      const response = await apiKeysApi.create({ name: name.trim() });
      setCreatedKey(response.api_key);
      setName('');
      await loadKeys();
    } catch (err) {
      Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'apiKeys.createFailed' }));
    } finally {
      setIsCreating(false);
    }
  };

  const handleRevoke = (key: ApiKeyInfo) => {
    Alert.alert(t('apiKeys.revokeTitle', { name: key.name }), t('apiKeys.revokeMessage'), [
      { text: t('common.cancel'), style: 'cancel' },
      {
        text: t('apiKeys.revoke'),
        style: 'destructive',
        onPress: () => {
          void (async () => {
            try {
              setRevokingKeyId(key.id);
              await apiKeysApi.revoke(key.id);
              setKeys((prev) => prev.filter((entry) => entry.id !== key.id));
            } catch (err) {
              Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'apiKeys.revokeFailed' }));
            } finally {
              setRevokingKeyId(null);
            }
          })();
        },
      },
    ]);
  };

  const openCreate = () => setShowCreate(true);
  const closeCreate = () => {
    setShowCreate(false);
    setCreatedKey(null);
  };

  const hasKeys = !isLoading && keys.length > 0;

  return (
    <View className="flex-1 bg-background-primary" testID="api-keys-screen">
      <PaneScrollView contentContainerStyle={{ paddingTop: spacing.lg, paddingBottom: spacing.xl }}>
        <Text className="text-sm text-text-secondary px-4 pb-6">{t('apiKeys.blurb')}</Text>

        {loadError && (
          <View className="flex-row flex-wrap items-baseline px-4 pb-3" testID="api-keys-load-error">
            <Text className="text-sm text-error">{loadError}</Text>
            <Text
              className="text-sm text-primary font-medium ml-1"
              accessibilityRole="button"
              onPress={() => { void loadKeys(); }}
              testID="api-keys-retry"
            >
              {t('common.retry')}
            </Text>
          </View>
        )}

        <Section
          title={t('settingsTabs.apiKeys')}
          testID="api-key-list"
          actions={
            hasKeys ? (
              <Text
                className="text-sm font-medium text-primary"
                accessibilityRole="button"
                onPress={openCreate}
                testID="new-api-key-button"
              >
                {t('apiKeys.create')}
              </Text>
            ) : undefined
          }
        >
          {isLoading ? (
            <View className="py-6 items-center">
              <ActivityIndicator size="small" color={colors.text.primary} />
            </View>
          ) : keys.length === 0 ? (
            <EmptyState
              testID="api-key-empty"
              action={{ label: t('apiKeys.create'), onPress: openCreate, testID: 'new-api-key-button' }}
            >
              {t('apiKeys.empty')}
            </EmptyState>
          ) : (
            keys.map((key, index) => (
              <Row
                key={key.id}
                testID={`api-key-row-${key.id}`}
                title={key.name}
                value={
                  requests[key.id] === undefined
                    ? `${key.key_prefix}…`
                    : `${key.key_prefix}… · ${t('apiKeys.requests30d', { count: requests[key.id] })}`
                }
                last={index === keys.length - 1}
                trailing={
                  revokingKeyId === key.id ? (
                    <ActivityIndicator size="small" color={colors.text.tertiary} />
                  ) : (
                    <Text
                      className="text-sm font-medium text-primary"
                      accessibilityRole="button"
                      onPress={() => handleRevoke(key)}
                      testID={`revoke-api-key-${key.id}`}
                    >
                      {t('apiKeys.revoke')}
                    </Text>
                  )
                }
              />
            ))
          )}
        </Section>
      </PaneScrollView>

      <Sheet visible={showCreate} onClose={closeCreate} testID="create-api-key-sheet">
        <Text className="text-lg font-semibold text-text-primary mb-4">
          {createdKey ? t('apiKeys.createdTitle') : t('apiKeys.create')}
        </Text>

        {createdKey ? (
          <>
            <Text className="text-sm text-text-secondary mb-3">{t('apiKeys.createdHint')}</Text>
            <View className="bg-surface-container-low rounded-lg p-3 mb-6">
              <Text className="text-sm font-mono text-text-primary" selectable testID="created-api-key-value">
                {createdKey}
              </Text>
            </View>
            <Button title={t('apiKeys.done')} onPress={closeCreate} fullWidth testID="api-key-created-done" />
          </>
        ) : (
          <>
            <Input
              label={t('apiKeys.nameLabel')}
              placeholder={t('apiKeys.namePlaceholder')}
              value={name}
              onChangeText={setName}
              testID="new-api-key-name"
            />
            <View className="flex-row gap-3 mt-6">
              <View className="flex-1">
                <Button title={t('common.cancel')} variant="ghost" onPress={closeCreate} fullWidth testID="create-api-key-cancel" />
              </View>
              <View className="flex-1">
                <Button
                  title={t('apiKeys.create')}
                  onPress={() => { void handleCreate(); }}
                  loading={isCreating}
                  disabled={!name.trim()}
                  fullWidth
                  testID="create-api-key-confirm"
                />
              </View>
            </View>
          </>
        )}
      </Sheet>
    </View>
  );
}
