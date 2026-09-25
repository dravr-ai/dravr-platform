// ABOUTME: API Tokens pane — one Section of 52 rows, the athlete's MCP bearer tokens with their mono prefix and an ink Revoke
// ABOUTME: Minting happens in the one bottom Sheet; the header's ink "New token" opens it (Boreal v2.2 Phase 4, api_tokens flag)

import React, { useCallback, useEffect, useState } from 'react';
import { View, Text, ActivityIndicator, Alert } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import { spacing, useThemeColors } from '../../constants/theme';
import { Button, EmptyState, Input, PaneScrollView, Row, Section, Sheet } from '../../components/ui';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import type { McpToken } from '../../types';
import { describeApiError } from '@pierre/ui-logic';

/**
 * Manage the MCP tokens this account has minted.
 *
 * A minted token is a long-lived bearer credential for the athlete's whole
 * fitness history, so the surface that creates them also has to be able to
 * take them back — mint-without-revoke is how a leaked token becomes
 * permanent.
 */
export function TokensScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { isAuthenticated } = useAuth();

  const [tokens, setTokens] = useState<McpToken[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [revokingTokenId, setRevokingTokenId] = useState<string | null>(null);
  const [showCreateToken, setShowCreateToken] = useState(false);
  const [newTokenName, setNewTokenName] = useState('');
  const [isCreatingToken, setIsCreatingToken] = useState(false);
  const [newToken, setNewToken] = useState<string | null>(null);

  const loadTokens = useCallback(async () => {
    try {
      setLoadError(null);
      const response = await userApi.getMcpTokens();
      const tokenList = response.tokens || [];
      const seen = new Set<string>();
      const deduplicated = tokenList.filter((token: { id: string; is_revoked: boolean }) => {
        if (token.is_revoked || seen.has(token.id)) return false;
        seen.add(token.id);
        return true;
      });
      setTokens(deduplicated);
    } catch (err) {
      setLoadError(describeApiError(err, { t, fallbackKey: 'app.failedLoadTokens' }));
      setTokens([]);
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    if (isAuthenticated) {
      void loadTokens();
    }
  }, [isAuthenticated, loadTokens]);

  const handleCreateToken = async () => {
    if (!newTokenName.trim()) {
      Alert.alert(t('common.error'), t('app.pleaseEnterTokenName'));
      return;
    }
    try {
      setIsCreatingToken(true);
      const token = await userApi.createMcpToken({
        name: newTokenName.trim(),
        expires_in_days: 365,
      });
      setNewToken(token.token_value || t('app.tokenCreatedBody'));
      await loadTokens();
      setNewTokenName('');
    } catch {
      Alert.alert(t('common.error'), t('app.failedCreateToken'));
    } finally {
      setIsCreatingToken(false);
    }
  };

  const handleRevokeToken = (token: McpToken) => {
    Alert.alert(
      t('app.revokeTokenTitle'),
      t('app.confirmRevokeToken', { token: token.name }),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.revoke'),
          style: 'destructive',
          onPress: () => {
            void (async () => {
              try {
                setRevokingTokenId(token.id);
                await userApi.revokeMcpToken(token.id);
                setTokens((prev) => prev.filter((entry) => entry.id !== token.id));
              } catch {
                Alert.alert(t('common.error'), t('app.failedRevokeToken'));
              } finally {
                setRevokingTokenId(null);
              }
            })();
          },
        },
      ],
    );
  };

  const openCreateSheet = () => setShowCreateToken(true);
  const closeCreateSheet = () => {
    setShowCreateToken(false);
    setNewToken(null);
  };

  // One `new-token-button` only: it sits in the Section header while there
  // are tokens, and on the empty state's ink action when there are none.
  const hasTokens = !isLoading && tokens.length > 0;

  return (
    <View className="flex-1 bg-background-primary" testID="tokens-screen">
      <PaneScrollView contentContainerStyle={{ paddingTop: spacing.lg, paddingBottom: spacing.xl }}>
        <Text className="text-sm text-text-secondary px-4 pb-6">{t('app.mcpTokenBlurb')}</Text>

        {loadError && (
          // The retry is a sibling, not a nested span: Android gives a nested
          // `Text` no native view, so a tap on it would reach nothing there.
          <View className="flex-row flex-wrap items-baseline px-4 pb-3" testID="tokens-load-error">
            <Text className="text-sm text-error">{loadError}</Text>
            <Text
              className="text-sm text-primary font-medium ml-1"
              accessibilityRole="button"
              onPress={() => { void loadTokens(); }}
              testID="tokens-retry"
            >
              {t('common.retry')}
            </Text>
          </View>
        )}

        <Section
          title={t('tokens.title')}
          testID="mcp-token-list"
          actions={
            hasTokens ? (
              <Text
                className="text-sm font-medium text-primary"
                accessibilityRole="button"
                onPress={openCreateSheet}
                testID="new-token-button"
              >
                {t('app.newToken')}
              </Text>
            ) : undefined
          }
        >
          {isLoading ? (
            <View className="py-6 items-center">
              <ActivityIndicator size="small" color={colors.text.primary} />
            </View>
          ) : tokens.length === 0 ? (
            <EmptyState
              testID="mcp-token-empty"
              action={{ label: t('app.newToken'), onPress: openCreateSheet, testID: 'new-token-button' }}
            >
              {t('tokens.empty')}
            </EmptyState>
          ) : (
            tokens.map((token, index) => (
              <Row
                key={token.id}
                testID={`mcp-token-row-${token.id}`}
                title={token.name}
                // The prefix-and-usage line is the one the row always carried,
                // in mono because the prefix is compared against a config file.
                value={t('app.tokenPrefixUsage', { prefix: token.token_prefix, uses: token.usage_count })}
                last={index === tokens.length - 1}
                trailing={
                  revokingTokenId === token.id ? (
                    <ActivityIndicator size="small" color={colors.text.tertiary} />
                  ) : (
                    <Text
                      className="text-sm font-medium text-primary"
                      accessibilityRole="button"
                      onPress={() => handleRevokeToken(token)}
                      testID={`revoke-token-${token.id}`}
                    >
                      {t('app.revoke')}
                    </Text>
                  )
                }
              />
            ))
          )}
        </Section>
      </PaneScrollView>

      <Sheet visible={showCreateToken} onClose={closeCreateSheet} testID="create-token-sheet">
        <Text className="text-lg font-semibold text-text-primary mb-4">
          {newToken ? t('app.tokenCreatedTitle') : t('app.createMcpToken')}
        </Text>

        {newToken ? (
          <>
            <Text className="text-sm text-text-secondary mb-3">{t('app.copyTokenNow')}</Text>
            <View className="bg-surface-container-low rounded-lg p-3 mb-6">
              <Text className="text-sm font-mono text-text-primary" selectable testID="created-token-value">
                {newToken}
              </Text>
            </View>
            <Button title={t('app.done')} onPress={closeCreateSheet} fullWidth testID="token-created-done" />
          </>
        ) : (
          <>
            <Input
              label={t('app.tokenName')}
              placeholder={t('app.tokenNamePlaceholder')}
              value={newTokenName}
              onChangeText={setNewTokenName}
              testID="new-token-name"
            />
            <View className="flex-row gap-3 mt-6">
              <View className="flex-1">
                <Button title={t('common.cancel')} variant="ghost" onPress={closeCreateSheet} fullWidth testID="create-token-cancel" />
              </View>
              <View className="flex-1">
                <Button
                  title={t('app.create')}
                  onPress={() => { void handleCreateToken(); }}
                  loading={isCreatingToken}
                  fullWidth
                  testID="create-token-confirm"
                />
              </View>
            </View>
          </>
        )}
      </Sheet>
    </View>
  );
}
