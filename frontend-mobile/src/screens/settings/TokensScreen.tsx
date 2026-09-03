// ABOUTME: API Tokens pane — list the athlete's MCP bearer tokens, mint one, take one back
// ABOUTME: Mobile counterpart of the web API Tokens pane, on the same api_tokens feature flag

import React, { useCallback, useEffect, useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  Modal,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import { spacing, borderRadius, useThemeColors } from '../../constants/theme';
import { Input, PaneScrollView } from '../../components/ui';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import type { McpToken } from '../../types';

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
  const router = useRouter();
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
      setLoadError(err instanceof Error ? err.message : t('app.failedLoadTokens'));
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

  const cardStyle: ViewStyle = {
    backgroundColor: colors.background.tertiary,
    borderWidth: 1,
    borderColor: colors.border.default,
    borderRadius: 16,
    overflow: 'hidden',
  };

  return (
    <SafeAreaView
      style={{ flex: 1, backgroundColor: colors.background.primary }}
      edges={['top']}
      testID="tokens-screen"
    >
      <View style={{ flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.md, paddingVertical: spacing.sm }}>
        <TouchableOpacity onPress={() => router.back()} testID="back-button" style={{ padding: 8, marginRight: 8 }}>
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text style={{ fontSize: 20, fontWeight: '600', color: colors.text.primary }}>{t('app.mcpTokens')}</Text>
      </View>

      <PaneScrollView contentContainerStyle={{ padding: spacing.md, gap: spacing.md }}>
        <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t('app.mcpTokenBlurb')}</Text>

        {loadError && (
          <View style={{ ...cardStyle, padding: 16, gap: 12 }} testID="tokens-load-error">
            <Text style={{ color: colors.pierre.red }}>{loadError}</Text>
            <TouchableOpacity onPress={() => { void loadTokens(); }} testID="tokens-retry">
              <Text style={{ color: colors.pierre.violet, fontWeight: '600' }}>{t('common.retry')}</Text>
            </TouchableOpacity>
          </View>
        )}

        <View style={cardStyle} testID="mcp-token-list">
          {isLoading ? (
            <View style={{ paddingVertical: 24, alignItems: 'center' }}>
              <ActivityIndicator size="small" color={colors.pierre.violet} />
            </View>
          ) : tokens.length === 0 ? (
            <Text
              style={{ padding: 24, textAlign: 'center', color: colors.text.tertiary }}
              testID="mcp-token-empty"
            >
              {t('app.noActiveTokens')}
            </Text>
          ) : (
            tokens.map((token, index) => (
              <View
                key={token.id}
                style={[
                  {
                    flexDirection: 'row',
                    alignItems: 'center',
                    paddingVertical: 14,
                    paddingHorizontal: 16,
                  },
                  index < tokens.length - 1
                    ? { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }
                    : {},
                ]}
                testID={`mcp-token-row-${token.id}`}
              >
                <View style={{ flex: 1, paddingRight: 12 }}>
                  <Text style={{ fontSize: 16, color: colors.text.primary }} numberOfLines={1}>
                    {token.name}
                  </Text>
                  <Text style={{ fontSize: 12, color: colors.text.tertiary }}>
                    {t('app.tokenPrefixUsage', { prefix: token.token_prefix, uses: token.usage_count })}
                  </Text>
                </View>
                <TouchableOpacity
                  onPress={() => handleRevokeToken(token)}
                  disabled={revokingTokenId === token.id}
                  testID={`revoke-token-${token.id}`}
                  style={{ paddingHorizontal: 12, paddingVertical: 8 }}
                >
                  {revokingTokenId === token.id ? (
                    <ActivityIndicator size="small" color={colors.pierre.red} />
                  ) : (
                    <Text style={{ color: colors.pierre.red, fontWeight: '600' }}>{t('app.revoke')}</Text>
                  )}
                </TouchableOpacity>
              </View>
            ))
          )}
        </View>

        <TouchableOpacity
          onPress={() => setShowCreateToken(true)}
          testID="new-token-button"
          style={{
            backgroundColor: colors.pierre.violet,
            borderRadius: 12,
            paddingVertical: 14,
            alignItems: 'center',
          }}
        >
          <Text style={{ fontSize: 16, fontWeight: '600', color: colors.tokens.onPrimary }}>
            {t('app.newToken')}
          </Text>
        </TouchableOpacity>
      </PaneScrollView>

      <Modal
        visible={showCreateToken}
        animationType="slide"
        transparent
        onRequestClose={() => setShowCreateToken(false)}
      >
        <View className="flex-1 bg-black/70 justify-center" style={{ paddingHorizontal: spacing.lg }}>
          <View className="bg-surface-container-low p-5" style={{ borderRadius: borderRadius.xl }}>
            <Text className="text-xl font-semibold text-on-surface mb-5 text-center">
              {newToken ? t('app.tokenCreatedTitle') : t('app.createMcpToken')}
            </Text>

            {newToken ? (
              <>
                <Text className="text-sm text-amber-500 text-center mb-3">{t('app.copyTokenNow')}</Text>
                <View className="bg-surface rounded-lg p-3 mb-5">
                  <Text className="text-sm text-on-surface font-mono" selectable>
                    {newToken}
                  </Text>
                </View>
                <TouchableOpacity
                  className="py-3 rounded-full items-center"
                  style={{ backgroundColor: colors.pierre.violet }}
                  onPress={() => {
                    setShowCreateToken(false);
                    setNewToken(null);
                  }}
                  testID="token-created-done"
                >
                  <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>
                    {t('app.done')}
                  </Text>
                </TouchableOpacity>
              </>
            ) : (
              <>
                <Input
                  label={t('app.tokenName')}
                  placeholder={t('app.tokenNamePlaceholder')}
                  value={newTokenName}
                  onChangeText={setNewTokenName}
                />
                <View className="flex-row gap-3 mt-4">
                  <TouchableOpacity
                    className="flex-1 py-3 rounded-full items-center"
                    style={{ backgroundColor: colors.background.tertiary }}
                    onPress={() => setShowCreateToken(false)}
                  >
                    <Text className="text-base font-semibold text-on-surface">{t('common.cancel')}</Text>
                  </TouchableOpacity>
                  <TouchableOpacity
                    className="flex-1 py-3 rounded-full items-center"
                    style={{ backgroundColor: colors.pierre.violet }}
                    onPress={() => { void handleCreateToken(); }}
                    disabled={isCreatingToken}
                    testID="create-token-confirm"
                  >
                    {isCreatingToken ? (
                      <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
                    ) : (
                      <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>
                        {t('app.create')}
                      </Text>
                    )}
                  </TouchableOpacity>
                </View>
              </>
            )}
          </View>
        </View>
      </Modal>
    </SafeAreaView>
  );
}
