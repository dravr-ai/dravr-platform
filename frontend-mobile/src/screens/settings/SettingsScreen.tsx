// ABOUTME: Profile & Settings screen with Stitch UX design
// ABOUTME: Shows profile header, stats, connected services, and settings sections

import React, { useState, useEffect, useCallback, useMemo } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  Alert,
  Modal,
  ActivityIndicator,
  Linking,
  type ViewStyle,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useFocusEffect } from 'expo-router';
import { useRouter } from 'expo-router';
import { LinearGradient } from 'expo-linear-gradient';
import { Feather } from '@expo/vector-icons';
import { spacing, borderRadius, useThemeColors, useTheme } from '../../constants/theme';
import type { AppearancePref } from '../../hooks/useAppearancePref';
import { Input } from '../../components/ui';
import { LanguageSwitcher } from '../../components/LanguageSwitcher';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../../contexts/AuthContext';
import { userApi, oauthApi } from '../../services/api';
import { useUsageStatus, type LimitCheckResult } from '../chat/useUsageStatus';
import { useFeatureFlags, FEATURE_KEYS } from '../../hooks/useFeatureFlags';
import type { McpToken, ExtendedProviderStatus } from '../../types';
import { BILLING_ENABLED } from '../../constants/features';
// Destinations for the About rows. Same targets the web Settings screen links
// to, so the two surfaces cannot drift to different help or terms pages.
const HELP_CENTER_URL = 'https://dravr.ai/help';
const TERMS_PRIVACY_URL = 'https://dravr.ai/privacy';

/**
 * Open an external URL, telling the user when the device cannot.
 *
 * A bare `Linking.openURL` rejects on a device with no handler, and an
 * unhandled rejection here would look identical to the row doing nothing —
 * which is the defect these rows already had.
 */
async function openExternal(url: string, t: (key: string, opts?: Record<string, unknown>) => string): Promise<void> {
  try {
    await Linking.openURL(url);
  } catch {
    // Module scope has no hook, so the caller — which is inside the component —
    // hands its `t` down rather than this reaching for one it cannot have.
    Alert.alert(t('app.couldNotOpenLink'), t('app.openInBrowserInstead', { url }));
  }
}

// Settings row style
const settingsRowStyle: ViewStyle = {
  flexDirection: 'row',
  alignItems: 'center',
  paddingVertical: 14,
  paddingHorizontal: 16,
};

/** Format large numbers compactly (e.g. 145000 -> "145.0K") */
function formatCompactNumber(value: number): string {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(1)}K`;
  }
  return value.toLocaleString();
}

/** Return hex color based on usage percentage: green < 70%, amber 70-90%, red > 90% */
function getUsageBarColor(
  current: number,
  limit: number,
  palette: { activity: string; nutrition: string; red: string },
): string {
  if (limit <= 0) return palette.activity;
  const pct = (current / limit) * 100;
  if (pct > 90) return palette.red;
  if (pct > 70) return palette.nutrition;
  return palette.activity;
}

/** Format ISO 8601 reset time in user's local timezone */
function formatResetTime(isoString: string): string {
  try {
    const date = new Date(isoString);
    return new Intl.DateTimeFormat(undefined, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(date);
  } catch {
    return 'midnight UTC';
  }
}

export function SettingsScreen() {
  const router = useRouter();
  const { user, logout, isAuthenticated } = useAuth();
  // Admins are platform operators: provider connections are a user-account
  // surface (mirrors the web's ADMIN_HIDDEN_TABS), so the Data section is
  // hidden for them. Gate on `role` to stay consistent with the web Dashboard.
  const isAdminUser = user?.role === 'admin' || user?.role === 'super_admin';
  const insets = useSafeAreaInsets();
  const colors = useThemeColors();
  const { pref: appearancePref, setPref: setAppearancePref } = useTheme();
  const { t } = useTranslation();

  // Glassmorphism card style — derived per render so it tracks the active scheme.
  const glassCardStyle: ViewStyle = {
    backgroundColor: colors.background.tertiary,
    borderWidth: 1,
    borderColor: colors.border.default,
    borderRadius: 16,
  };
  // `api_tokens` is the gate the web Settings screen already applies to its API
  // Tokens tab. Mobile adopts the same key rather than minting long-lived MCP
  // bearer credentials for every user by default.
  const { flags: featureFlags } = useFeatureFlags();
  const apiTokensEnabled = featureFlags[FEATURE_KEYS.apiTokens];
  const [tokens, setTokens] = useState<McpToken[]>([]);
  const [showTokenManager, setShowTokenManager] = useState(false);
  const [showCreateToken, setShowCreateToken] = useState(false);
  const [revokingTokenId, setRevokingTokenId] = useState<string | null>(null);
  const [showChangePassword, setShowChangePassword] = useState(false);
  const [newTokenName, setNewTokenName] = useState('');
  const [isCreatingToken, setIsCreatingToken] = useState(false);
  const [newToken, setNewToken] = useState<string | null>(null);
  const [connectedProviders, setConnectedProviders] = useState<ExtendedProviderStatus[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Password change state
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [isChangingPassword, setIsChangingPassword] = useState(false);

  // Reload provider status when screen comes into focus (e.g., after OAuth connection)
  useFocusEffect(
    useCallback(() => {
      if (isAuthenticated) {
        loadProviderStatus();
      }
    }, [isAuthenticated])
  );

  // Memoised because it now closes over `t`, which makes it a reactive value:
  // the effect below calls it, and without this the effect would re-run on
  // every render. The filter parameter is `token`, not `t` — it shadowed the
  // translator, which was harmless while the catch block held the only t() call
  // and would not have stayed harmless.
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
      const errorMessage = err instanceof Error ? err.message : t('app.failedLoadTokens');
      setLoadError(errorMessage);
      console.error('Failed to load tokens:', err);
      setTokens([]);
    }
  }, [t]);

  useEffect(() => {
    if (isAuthenticated && apiTokensEnabled) {
      loadTokens();
    }
  }, [isAuthenticated, apiTokensEnabled, loadTokens]);

  const loadProviderStatus = async () => {
    try {
      // Use getProvidersStatus() to include non-OAuth providers like synthetic
      const response = await oauthApi.getProvidersStatus();
      setConnectedProviders(response.providers || []);
    } catch (err) {
      console.error('Failed to load provider status:', err);
    }
  };

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

  /**
   * Revoke one MCP token after an explicit confirm.
   *
   * A minted token is a long-lived bearer credential for the athlete's whole
   * fitness history, so the mobile surface that creates them has to be able to
   * take them back — the confirm mirrors the destructive-action pattern the
   * rest of this screen uses for sign-out and provider disconnects.
   */
  const handleRevokeToken = (token: McpToken) => {
    Alert.alert(
      t('app.revokeTokenTitle'),
      `Revoke "${token.name}"? Any client still using it loses access immediately.`,
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.revoke'),
          style: 'destructive',
          onPress: async () => {
            try {
              setRevokingTokenId(token.id);
              await userApi.revokeMcpToken(token.id);
              setTokens((prev) => prev.filter((t) => t.id !== token.id));
            } catch {
              Alert.alert(t('common.error'), t('app.failedRevokeToken'));
            } finally {
              setRevokingTokenId(null);
            }
          },
        },
      ]
    );
  };

  const handleChangePassword = async () => {
    if (!currentPassword || !newPassword || !confirmPassword) {
      Alert.alert(t('common.error'), t('app.pleaseFillAllFields'));
      return;
    }

    if (newPassword !== confirmPassword) {
      Alert.alert(t('common.error'), t('app.newPasswordsMismatch'));
      return;
    }

    if (newPassword.length < 8) {
      Alert.alert(t('common.error'), t('app.passwordTooShort'));
      return;
    }

    try {
      setIsChangingPassword(true);
      await userApi.changePassword(currentPassword, newPassword);
      Alert.alert(t('common.success'), t('app.passwordChanged'));
      setShowChangePassword(false);
      setCurrentPassword('');
      setNewPassword('');
      setConfirmPassword('');
    } catch {
      Alert.alert(t('common.error'), t('app.failedChangePasswordCheck'));
    } finally {
      setIsChangingPassword(false);
    }
  };

  const handleLogout = () => {
    Alert.alert(
      t('common.logout'),
      t('app.signOutConfirm'),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('common.logout'),
          style: 'destructive',
          onPress: logout,
        },
      ]
    );
  };

  // Usage quota status
  const { data: usageData, isLoading: usageLoading } = useUsageStatus();

  const usageBars = useMemo(() => {
    if (!usageData) return [];
    return [
      { label: t('app.dailyMessages'), counter: usageData.daily.messages, compact: false },
      { label: t('app.dailyTokens'), counter: usageData.daily.tokens, compact: true },
      { label: t('app.weeklyMessages'), counter: usageData.weekly.messages, compact: false },
    ] as { label: string; counter: LimitCheckResult; compact: boolean }[];
  }, [usageData, t]);

  const displayName = user?.display_name || user?.email?.split('@')[0] || t('app.athlete');

  return (
    <View style={{ flex: 1, backgroundColor: colors.background.primary }} testID="settings-screen">
      <ScrollView
        style={{ flex: 1 }}
        contentContainerStyle={{
          paddingTop: insets.top + spacing.sm,
          paddingBottom: 100,
          paddingHorizontal: spacing.md,
        }}
        showsVerticalScrollIndicator={false}
      >
        {/* Profile Header with gradient-bordered avatar */}
        <View style={{ alignItems: 'center', paddingHorizontal: 16, paddingVertical: 24 }} testID="settings-profile-section">
          {/* Load Error Display */}
          {loadError && (
            <View className="w-full mb-4 p-3 bg-error/10 border border-error/30 rounded-lg flex-row items-center justify-between">
              <Text className="flex-1 text-error text-sm mr-3">{loadError}</Text>
              <TouchableOpacity
                className="px-3 py-1.5 bg-error/20 rounded-md"
                onPress={() => {
                  setLoadError(null);
                  loadTokens();
                }}
              >
                <Text className="text-error text-sm font-semibold">{t('common.retry')}</Text>
              </TouchableOpacity>
            </View>
          )}
          {/* Gradient-bordered Avatar */}
          <LinearGradient
            colors={[colors.pierre.violet, colors.pierre.cyan]}
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 1 }}
            style={{
              width: 112,
              height: 112,
              borderRadius: 56,
              alignItems: 'center',
              justifyContent: 'center',
              marginBottom: 16,
              padding: 4,
            }}
          >
            <View style={{
              width: '100%',
              height: '100%',
              borderRadius: 56,
              backgroundColor: colors.background.primary,
              alignItems: 'center',
              justifyContent: 'center',
            }}>
              <Text style={{ fontSize: 36, fontWeight: 'bold', color: colors.text.primary }}>
                {displayName[0]?.toUpperCase() || 'U'}
              </Text>
            </View>
          </LinearGradient>

          <Text style={{ fontSize: 24, fontWeight: 'bold', color: colors.text.primary, marginBottom: 4 }}>{displayName}</Text>
          <Text style={{ fontSize: 16, color: colors.text.tertiary, marginBottom: 16 }}>{user?.email}</Text>

          {/* Edit Profile Button with violet glow */}
          <TouchableOpacity
            style={{
              paddingHorizontal: 24,
              paddingVertical: 10,
              borderRadius: 9999,
              backgroundColor: colors.pierre.violet,
              shadowColor: colors.pierre.violet,
              shadowOffset: { width: 0, height: 0 },
              shadowOpacity: 0.4,
              shadowRadius: 12,
              elevation: 6,
            }}
            onPress={() => router.push('/(app)/(tabs)/(settings)/profile')}
            testID="settings-edit-profile-button"
          >
            <Text style={{ fontSize: 14, fontWeight: '600', color: colors.tokens.onPrimary }}>{t('app.editProfile')}</Text>
          </TouchableOpacity>
        </View>

        {/* Data Providers Section - navigates to Connections screen */}
        {!isAdminUser && (
          <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-data-section">
            <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>{t('app.data')}</Text>
            <View style={glassCardStyle}>
              <TouchableOpacity
                style={settingsRowStyle}
                onPress={() => router.push('/(app)/(tabs)/(settings)/connections')}
                testID="settings-data-providers-button"
              >
                <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                  <Feather name="link" size={20} color={colors.text.secondary} />
                </View>
                <View style={{ flex: 1 }}>
                  <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('app.dataProviders')}</Text>
                  <Text style={{ fontSize: 14, color: colors.text.tertiary }}>
                    {connectedProviders.filter(p => p.connected).length} connected
                  </Text>
                </View>
                <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
              </TouchableOpacity>
            </View>
          </View>
        )}

        {/* Coaching Style Section */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-coaching-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>{t('app.coaching')}</Text>
          <View style={glassCardStyle}>
            <TouchableOpacity
              style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}
              onPress={() => router.push('/(app)/(tabs)/(settings)/coaching-style')}
              testID="settings-coaching-style-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="message-square" size={20} color={colors.text.secondary} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('app.coachingStyle')}</Text>
                <Text style={{ fontSize: 14, color: colors.text.tertiary, textTransform: 'capitalize' }}>
                  {(user?.coaching_persona ?? 'casual').replace('_', '-')}
                </Text>
              </View>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            {/* Onboarding could link a chat app but nothing could show or undo
                it afterwards. Web exposes this under Settings; mobile now does
                too. */}
            <TouchableOpacity
              style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}
              onPress={() => router.push('/(app)/(tabs)/(settings)/messaging')}
              testID="settings-messaging-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="message-circle" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>{t('app.messaging')}</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            {/* Bring-your-own AI key. Web exposes this under Settings; without
                it a mobile-only user cannot see which provider is serving them
                or supply their own. */}
            <TouchableOpacity
              style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}
              onPress={() => router.push('/(app)/(tabs)/(settings)/ai-provider')}
              testID="settings-ai-provider-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="cpu" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>{t('app.aiProvider')}</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            {/* Notification preferences. The GET/PUT endpoints, the api-client
                method and this hook all existed; nothing rendered them, so a
                muted category could only be set from another client. */}
            <TouchableOpacity
              style={settingsRowStyle}
              onPress={() => router.push('/(app)/(tabs)/(settings)/notification-preferences')}
              testID="settings-notifications-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="bell" size={20} color={colors.text.secondary} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('common.notifications')}</Text>
                <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t('app.notifPrefsSubtitle')}</Text>
              </View>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>
        </View>

        {/* Account Settings Section */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-account-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>{t('app.account')}</Text>
          <View style={glassCardStyle}>
            {/* Same destination as the header's Edit Profile. Web exposes a
                single Profile tab, so two rows leading to two different places
                would be the drift rather than the parity. */}
            <TouchableOpacity
              style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}
              onPress={() => router.push('/(app)/(tabs)/(settings)/profile')}
              testID="settings-personal-info-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="user" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>{t('app.personalInformation')}</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            <TouchableOpacity
              style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}
              onPress={() => setShowChangePassword(true)}
              testID="settings-change-password-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="lock" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>{t('app.changePassword')}</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            {/* Gated on `api_tokens`, the same flag the web Settings screen uses
                for its API Tokens tab. It defaults off, so neither surface hands
                out long-lived MCP bearer credentials unless a tenant turns it on. */}
            {apiTokensEnabled && (
              <TouchableOpacity
                style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}
                onPress={() => setShowTokenManager(true)}
                testID="settings-mcp-tokens-button"
              >
                <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                  <Feather name="key" size={20} color={colors.text.secondary} />
                </View>
                <View style={{ flex: 1 }}>
                  <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('app.mcpTokens')}</Text>
                  <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{tokens.length} active</Text>
                </View>
                <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
              </TouchableOpacity>
            )}

            <TouchableOpacity
              style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}
              onPress={() => router.push('/(app)/(tabs)/(settings)/connected-apps')}
              testID="settings-connected-apps-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="grid" size={20} color={colors.text.secondary} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('app.connectedApps')}</Text>
                <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t('app.externalMcpClients')}</Text>
              </View>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            {/* Memory — the screen and its route already existed but nothing
                navigated to them, so the inspector was unreachable in the app. */}
            <TouchableOpacity
              style={BILLING_ENABLED ? [settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }] : settingsRowStyle}
              onPress={() => router.push('/(app)/memory')}
              testID="settings-memory-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="cpu" size={20} color={colors.text.secondary} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('app.memory')}</Text>
                <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t('app.whatYourCoachRemembers')}</Text>
              </View>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            {/* Billing rides the same flag as the web Usage tab. The route
                itself also redirects when disabled, so this row is the entry
                point that was missing rather than a second gate. */}
            {BILLING_ENABLED && (
              <TouchableOpacity
                style={settingsRowStyle}
                onPress={() => router.push('/(app)/billing')}
                testID="settings-billing-button"
              >
                <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                  <Feather name="credit-card" size={20} color={colors.text.secondary} />
                </View>
                <View style={{ flex: 1 }}>
                  <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('app.billing')}</Text>
                  <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t('app.planAndUsage')}</Text>
                </View>
                <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
              </TouchableOpacity>
            )}
          </View>
        </View>

        {/* Appearance Section — System / Light / Dark */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-appearance-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>{t('settings.appearance')}</Text>
          <View style={glassCardStyle}>
            {(['system', 'dark', 'light'] as const).map((option, idx, arr) => {
              const isSelected = appearancePref === option;
              const label =
                option === 'system'
                  ? t('settings.appearanceSystem')
                  : option === 'dark'
                    ? t('settings.appearanceDark')
                    : t('settings.appearanceLight');
              const description =
                option === 'system'
                  ? t('settings.appearanceSystemHint')
                  : option === 'dark'
                    ? t('settings.appearanceDarkHint')
                    : t('settings.appearanceLightHint');
              const icon = option === 'system' ? 'smartphone' : option === 'dark' ? 'moon' : 'sun';
              const isLast = idx === arr.length - 1;
              return (
                <TouchableOpacity
                  key={option}
                  testID={`appearance-option-${option}`}
                  accessibilityRole="radio"
                  accessibilityState={{ selected: isSelected }}
                  onPress={() => {
                    void setAppearancePref(option as AppearancePref);
                  }}
                  style={[
                    settingsRowStyle,
                    !isLast && { borderBottomWidth: 1, borderBottomColor: colors.border.subtle },
                  ]}
                >
                  <View
                    style={{
                      width: 40,
                      height: 40,
                      borderRadius: 12,
                      backgroundColor: colors.background.secondary,
                      alignItems: 'center',
                      justifyContent: 'center',
                      marginRight: 12,
                    }}
                  >
                    <Feather name={icon} size={20} color={colors.text.secondary} />
                  </View>
                  <View style={{ flex: 1 }}>
                    <Text style={{ fontSize: 16, color: colors.text.primary }}>{label}</Text>
                    <Text style={{ fontSize: 13, color: colors.text.tertiary, marginTop: 2 }}>{description}</Text>
                  </View>
                  <Feather
                    name={isSelected ? 'check-circle' : 'circle'}
                    size={20}
                    color={isSelected ? colors.pierre.violet : colors.text.tertiary}
                  />
                </TouchableOpacity>
              );
            })}
          </View>
        </View>

        {/* Language Section — the switcher's only reachable home. It sets the
            chrome language AND `users.locale`, so the coach answers in the
            language the athlete reads the app in. */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-language-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 4 }}>{t('settings.language')}</Text>
          <Text style={{ fontSize: 13, color: colors.text.tertiary, marginBottom: 12 }}>{t('settings.languageDescription')}</Text>
          <LanguageSwitcher serverLocale={user?.locale} />
        </View>

        {/* Usage Section */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-usage-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>{t('app.usage')}</Text>
          <View style={glassCardStyle}>
            {usageLoading ? (
              <View style={{ paddingVertical: 24, alignItems: 'center' }}>
                <ActivityIndicator size="small" color={colors.pierre.violet} />
              </View>
            ) : !usageData ? (
              <View style={{ paddingVertical: 24, alignItems: 'center' }}>
                <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t('app.usageDataUnavailable')}</Text>
              </View>
            ) : (
              <View style={{ padding: 16 }}>
                {/* Progress bars */}
                {usageBars.map(({ label, counter, compact }) => {
                  const pct = counter.limit > 0 ? Math.min((counter.current / counter.limit) * 100, 100) : 0;
                  return (
                    <View key={label} style={{ marginBottom: 16 }}>
                      <View style={{ flexDirection: 'row', justifyContent: 'space-between', marginBottom: 6 }}>
                        <Text style={{ fontSize: 14, fontWeight: '500', color: colors.text.secondary }}>{label}</Text>
                        <Text style={{ fontSize: 14, color: colors.text.tertiary }}>
                          {compact ? formatCompactNumber(counter.current) : counter.current.toLocaleString()}
                          {' / '}
                          {compact ? formatCompactNumber(counter.limit) : counter.limit.toLocaleString()}
                        </Text>
                      </View>
                      <View style={{ height: 8, backgroundColor: colors.background.tertiary, borderRadius: 4, overflow: 'hidden' }}>
                        <View
                          style={{
                            height: '100%',
                            width: `${pct}%`,
                            backgroundColor: getUsageBarColor(counter.current, counter.limit, {
                              activity: colors.pierre.activity,
                              nutrition: colors.pierre.nutrition,
                              red: colors.pierre.red,
                            }),
                            borderRadius: 4,
                          }}
                        />
                      </View>
                    </View>
                  );
                })}

                {/* Reset time */}
                <Text style={{ fontSize: 12, color: colors.text.tertiary, marginBottom: 16 }}>
                  {t('app.dailyLimitsResetAt', {
                    time: formatResetTime(usageData.daily.messages.resets_at),
                  })}
                </Text>

                {/* Resource counts */}
                <View style={{ borderTopWidth: 1, borderTopColor: colors.border.default, paddingTop: 16 }}>
                  <View style={{ flexDirection: 'row', gap: 12 }}>
                    <View style={{ flex: 1, backgroundColor: colors.background.tertiary, borderRadius: 8, padding: 12 }}>
                      <Text style={{ fontSize: 12, color: colors.text.tertiary, marginBottom: 4 }}>{t('app.coaches')}</Text>
                      <Text style={{ fontSize: 14, fontWeight: '500', color: colors.text.primary }}>
                        {usageData.resources.coaches} / {usageData.resources.max_coaches}
                      </Text>
                    </View>
                    <View style={{ flex: 1, backgroundColor: colors.background.tertiary, borderRadius: 8, padding: 12 }}>
                      <Text style={{ fontSize: 12, color: colors.text.tertiary, marginBottom: 4 }}>{t('app.conversations')}</Text>
                      <Text style={{ fontSize: 14, fontWeight: '500', color: colors.text.primary }}>
                        {usageData.resources.conversations} / {usageData.resources.max_conversations}
                      </Text>
                    </View>
                  </View>
                </View>
              </View>
            )}
          </View>
        </View>

        {/* Privacy Section */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }}>
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>{t('app.privacy')}</Text>
          <View style={glassCardStyle}>
            {/* The privacy screen carries the analytics-consent control,
                matching what the web Privacy & Data tab covers. */}
            <TouchableOpacity
              style={settingsRowStyle}
              onPress={() => router.push('/(app)/(tabs)/(settings)/privacy')}
              testID="settings-privacy-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="shield" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>{t('app.privacySettings')}</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>
        </View>

        {/* About Section */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }}>
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>{t('common.about')}</Text>
          <View style={glassCardStyle}>
            {/* Informational only — rendered as a plain row so it does not
                advertise a tap it cannot service. */}
            <View style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]} testID="settings-version-row">
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="info" size={20} color={colors.text.secondary} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('app.version')}</Text>
                <Text style={{ fontSize: 14, color: colors.text.tertiary }}>1.0.0</Text>
              </View>
            </View>

            <TouchableOpacity
              style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}
              onPress={() => { void openExternal(HELP_CENTER_URL, t); }}
              testID="settings-help-center-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="help-circle" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>{t('app.helpCenter')}</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            <TouchableOpacity
              style={settingsRowStyle}
              onPress={() => { void openExternal(TERMS_PRIVACY_URL, t); }}
              testID="settings-terms-privacy-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="file-text" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>{t('app.termsAndPrivacy')}</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>
        </View>

        {/* Log Out Button - soft red */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }}>
          <TouchableOpacity
            style={[glassCardStyle, { borderColor: 'rgba(255, 107, 107, 0.3)', paddingVertical: 16, alignItems: 'center' }]}
            onPress={handleLogout}
            testID="settings-logout-button"
          >
            <Text style={{ fontSize: 16, fontWeight: '600', color: colors.pierre.red }}>{t('app.logOut')}</Text>
          </TouchableOpacity>
        </View>
      </ScrollView>

      {/* MCP Token Manager — the list the mint flow never had. Without it a
          minted bearer credential could not be taken back from the device that
          created it. */}
      <Modal
        visible={showTokenManager}
        animationType="slide"
        transparent
        onRequestClose={() => setShowTokenManager(false)}
      >
        <View
          className="flex-1 bg-black/70 justify-center"
          style={{ paddingHorizontal: spacing.lg }}
        >
          <View
            className="bg-surface-container-low p-5"
            style={{ borderRadius: borderRadius.xl, maxHeight: '80%' }}
            testID="mcp-token-manager"
          >
            <Text className="text-xl font-semibold text-on-surface mb-1 text-center">
              {t('app.mcpTokens')}
            </Text>
            <Text className="text-sm text-on-surface-variant mb-4 text-center">
              {t('app.mcpTokenBlurb')}
            </Text>

            {tokens.length === 0 ? (
              <Text
                className="text-sm text-on-surface-variant text-center py-6"
                testID="mcp-token-empty"
              >
                {t('app.noActiveTokens')}
              </Text>
            ) : (
              <ScrollView style={{ maxHeight: 320 }} testID="mcp-token-list">
                {tokens.map((token) => (
                  <View
                    key={token.id}
                    className="flex-row items-center py-3 border-b border-border-subtle"
                    testID={`mcp-token-row-${token.id}`}
                  >
                    <View className="flex-1 pr-3">
                      <Text className="text-base text-on-surface" numberOfLines={1}>
                        {token.name}
                      </Text>
                      <Text className="text-xs text-on-surface-variant font-mono mt-0.5">
                        {token.token_prefix}… · used {token.usage_count}×
                      </Text>
                    </View>
                    <TouchableOpacity
                      className="px-3 py-1.5 rounded-md bg-error/20"
                      onPress={() => handleRevokeToken(token)}
                      disabled={revokingTokenId === token.id}
                      testID={`revoke-token-${token.id}`}
                    >
                      {revokingTokenId === token.id ? (
                        <ActivityIndicator size="small" color={colors.error} />
                      ) : (
                        <Text className="text-error text-sm font-semibold">{t('app.revoke')}</Text>
                      )}
                    </TouchableOpacity>
                  </View>
                ))}
              </ScrollView>
            )}

            <View className="flex-row gap-3 mt-5">
              <TouchableOpacity
                className="flex-1 py-3 rounded-full items-center"
                style={{ backgroundColor: colors.background.tertiary }}
                onPress={() => setShowTokenManager(false)}
                testID="close-token-manager"
              >
                <Text className="text-base font-semibold text-on-surface">{t('common.close')}</Text>
              </TouchableOpacity>
              <TouchableOpacity
                className="flex-1 py-3 rounded-full items-center"
                style={{ backgroundColor: colors.pierre.violet }}
                onPress={() => {
                  setShowTokenManager(false);
                  setShowCreateToken(true);
                }}
                testID="new-token-button"
              >
                <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>
                  {t('app.newToken')}
                </Text>
              </TouchableOpacity>
            </View>
          </View>
        </View>
      </Modal>

      {/* Create Token Modal */}
      <Modal
        visible={showCreateToken}
        animationType="slide"
        transparent
        onRequestClose={() => setShowCreateToken(false)}
      >
        <View
          className="flex-1 bg-black/70 justify-center"
          style={{ paddingHorizontal: spacing.lg }}
        >
          <View
            className="bg-surface-container-low p-5"
            style={{ borderRadius: borderRadius.xl }}
          >
            <Text className="text-xl font-semibold text-on-surface mb-5 text-center">
              {newToken ? t('app.tokenCreatedTitle') : t('app.createMcpToken')}
            </Text>

            {newToken ? (
              <>
                <Text className="text-sm text-amber-500 text-center mb-3">
                  {t('app.copyTokenNow')}
                </Text>
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
                    // Back to the list, where the new token is now revocable.
                    setShowTokenManager(true);
                  }}
                  testID="token-created-done"
                >
                  <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>{t('app.done')}</Text>
                </TouchableOpacity>
              </>
            ) : (
              <>
                <Input
                  label={t('app.tokenName')}
                  placeholder="e.g., Claude Desktop"
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
                    onPress={handleCreateToken}
                    disabled={isCreatingToken}
                  >
                    {isCreatingToken ? (
                      <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
                    ) : (
                      <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>{t('app.create')}</Text>
                    )}
                  </TouchableOpacity>
                </View>
              </>
            )}
          </View>
        </View>
      </Modal>

      {/* Change Password Modal */}
      <Modal
        visible={showChangePassword}
        animationType="slide"
        transparent
        onRequestClose={() => setShowChangePassword(false)}
      >
        <View
          className="flex-1 bg-black/70 justify-center"
          style={{ paddingHorizontal: spacing.lg }}
        >
          <View
            className="bg-surface-container-low p-5"
            style={{ borderRadius: borderRadius.xl }}
          >
            <Text className="text-xl font-semibold text-on-surface mb-5 text-center">
              {t('app.changePassword')}
            </Text>

            <Input
              label={t('app.currentPassword')}
              value={currentPassword}
              onChangeText={setCurrentPassword}
              secureTextEntry
              showPasswordToggle
            />
            <Input
              label={t('app.newPassword')}
              value={newPassword}
              onChangeText={setNewPassword}
              secureTextEntry
              showPasswordToggle
            />
            <Input
              label={t('app.confirmNewPassword')}
              value={confirmPassword}
              onChangeText={setConfirmPassword}
              secureTextEntry
              showPasswordToggle
            />

            <View className="flex-row gap-3 mt-4">
              <TouchableOpacity
                className="flex-1 py-3 rounded-full items-center"
                style={{ backgroundColor: colors.background.tertiary }}
                onPress={() => setShowChangePassword(false)}
              >
                <Text className="text-base font-semibold text-on-surface">{t('common.cancel')}</Text>
              </TouchableOpacity>
              <TouchableOpacity
                className="flex-1 py-3 rounded-full items-center"
                style={{ backgroundColor: colors.pierre.violet }}
                onPress={handleChangePassword}
                disabled={isChangingPassword}
              >
                {isChangingPassword ? (
                  <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
                ) : (
                  <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>{t('app.change')}</Text>
                )}
              </TouchableOpacity>
            </View>
          </View>
        </View>
      </Modal>
    </View>
  );
}
