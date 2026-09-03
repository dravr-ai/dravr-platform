// ABOUTME: Account pane — status, usage, security, connected MCP apps and sign-out, in one place
// ABOUTME: Section order comes from the shared settings declaration, so web groups the same five

import React, { useMemo, useState } from 'react';
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
import { settingsPaneSections } from '@pierre/shared-constants';
import { spacing, borderRadius, useThemeColors } from '../../constants/theme';
import { Input, PaneScrollView } from '../../components/ui';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useUsageStatus, type LimitCheckResult } from '../chat/useUsageStatus';

/** Format large numbers compactly (e.g. 145000 -> "145.0K"). */
function formatCompactNumber(value: number): string {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(1)}K`;
  }
  return value.toLocaleString();
}

/** Green under 70% of the cap, amber to 90%, red past it. */
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

/**
 * `fallback` is the caller's translated wording for an unparseable timestamp:
 * this runs outside the component, so it cannot reach the catalogue itself.
 */
function formatResetTime(isoString: string, fallback: string): string {
  try {
    return new Intl.DateTimeFormat(undefined, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(new Date(isoString));
  } catch {
    return fallback;
  }
}

/** The account creation date, in the reader's own locale. */
function formatMemberSince(isoString: string | undefined, fallback: string): string {
  if (!isoString) return fallback;
  try {
    return new Intl.DateTimeFormat(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
    }).format(new Date(isoString));
  } catch {
    return fallback;
  }
}

/**
 * Everything about the account itself.
 *
 * Status, usage, security and the connected MCP apps belong together, and web
 * has held them together since it had panes. The section order is read from the
 * shared declaration rather than typed twice, which is what let the phone
 * scatter the same four things down one scroll with nothing failing.
 */
export function AccountScreen() {
  const { t } = useTranslation();
  const router = useRouter();
  const colors = useThemeColors();
  const { user, logout } = useAuth();

  const [showChangePassword, setShowChangePassword] = useState(false);
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [isChangingPassword, setIsChangingPassword] = useState(false);

  const { data: usageData, isLoading: usageLoading } = useUsageStatus();

  const usageBars = useMemo(() => {
    if (!usageData) return [];
    return [
      { label: t('app.dailyMessages'), counter: usageData.daily.messages, compact: false },
      { label: t('app.dailyTokens'), counter: usageData.daily.tokens, compact: true },
      { label: t('app.weeklyMessages'), counter: usageData.weekly.messages, compact: false },
    ] as { label: string; counter: LimitCheckResult; compact: boolean }[];
  }, [usageData, t]);

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
        { text: t('common.logout'), style: 'destructive', onPress: logout },
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

  const factRow = (label: string, value: string, isLast: boolean) => (
    <View
      key={label}
      style={[
        rowStyle,
        { justifyContent: 'space-between' },
        isLast ? {} : { borderBottomWidth: 1, borderBottomColor: colors.border.subtle },
      ]}
    >
      <Text style={{ fontSize: 15, color: colors.text.secondary }}>{label}</Text>
      <Text style={{ fontSize: 15, color: colors.text.primary }}>{value}</Text>
    </View>
  );

  const renderSection = (section: string) => {
    switch (section) {
      case 'account-status':
        return (
          <View key={section} testID="account-section-account-status">
            <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>
              {t('profile.accountStatus')}
            </Text>
            <View style={cardStyle}>
              {factRow(t('settingsUi.status'), user?.user_status ?? t('settingsUi.unknownDate'), false)}
              {factRow(t('settingsUi.role'), user?.role ?? t('settingsUi.unknownDate'), false)}
              {factRow(
                t('profile.memberSince'),
                formatMemberSince(user?.created_at, t('settingsUi.unknownDate')),
                true,
              )}
            </View>
          </View>
        );

      case 'usage':
        return (
          <View key={section} testID="account-section-usage">
            <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>
              {t('app.usage')}
            </Text>
            <View style={cardStyle}>
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

                  <Text style={{ fontSize: 12, color: colors.text.tertiary, marginBottom: 16 }}>
                    {t('app.dailyLimitsResetAt', {
                      time: formatResetTime(usageData.daily.messages.resets_at, t('settingsUi.midnightUtc')),
                    })}
                  </Text>

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
        );

      case 'security':
        return (
          <View key={section} testID="account-section-security">
            <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>
              {t('settingsUi.security')}
            </Text>
            <View style={cardStyle}>
              <TouchableOpacity
                style={rowStyle}
                onPress={() => setShowChangePassword(true)}
                testID="account-change-password-button"
              >
                <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                  <Feather name="lock" size={20} color={colors.text.secondary} />
                </View>
                <View style={{ flex: 1 }}>
                  <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('app.changePassword')}</Text>
                  <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t('password.changeHint')}</Text>
                </View>
                <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
              </TouchableOpacity>
            </View>
          </View>
        );

      case 'connected-mcp-apps':
        return (
          <View key={section} testID="account-section-connected-mcp-apps">
            <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>
              {t('tokens.connectedMcpApps')}
            </Text>
            <View style={cardStyle}>
              <TouchableOpacity
                style={rowStyle}
                onPress={() => router.push('/(app)/(tabs)/(settings)/connected-apps')}
                testID="account-connected-apps-button"
              >
                <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                  <Feather name="grid" size={20} color={colors.text.secondary} />
                </View>
                <View style={{ flex: 1 }}>
                  <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('app.connectedApps')}</Text>
                  <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t('tokens.connectedAppsHint')}</Text>
                </View>
                <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
              </TouchableOpacity>
            </View>
          </View>
        );

      case 'sign-out':
        return (
          <View key={section} testID="account-section-sign-out">
            <TouchableOpacity
              style={{
                ...cardStyle,
                borderColor: colors.pierre.red,
                paddingVertical: 16,
                alignItems: 'center',
              }}
              onPress={handleLogout}
              testID="account-logout-button"
            >
              <Text style={{ fontSize: 16, fontWeight: '600', color: colors.pierre.red }}>{t('app.logOut')}</Text>
            </TouchableOpacity>
            <Text style={{ fontSize: 13, color: colors.text.tertiary, marginTop: 8, textAlign: 'center' }}>
              {t('account.signOutHint')}
            </Text>
          </View>
        );

      default:
        return null;
    }
  };

  return (
    <SafeAreaView
      style={{ flex: 1, backgroundColor: colors.background.primary }}
      edges={['top']}
      testID="account-screen"
    >
      <View style={{ flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.md, paddingVertical: spacing.sm }}>
        <TouchableOpacity onPress={() => router.back()} testID="back-button" style={{ padding: 8, marginRight: 8 }}>
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text style={{ fontSize: 20, fontWeight: '600', color: colors.text.primary }}>{t('app.account')}</Text>
      </View>

      <PaneScrollView contentContainerStyle={{ padding: spacing.md, gap: spacing.lg }}>
        {settingsPaneSections('account').map(renderSection)}
      </PaneScrollView>

      <Modal
        visible={showChangePassword}
        animationType="slide"
        transparent
        onRequestClose={() => setShowChangePassword(false)}
      >
        <View className="flex-1 bg-black/70 justify-center" style={{ paddingHorizontal: spacing.lg }}>
          <View className="bg-surface-container-low p-5" style={{ borderRadius: borderRadius.xl }}>
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
                onPress={() => { void handleChangePassword(); }}
                disabled={isChangingPassword}
              >
                {isChangingPassword ? (
                  <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
                ) : (
                  <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>
                    {t('app.change')}
                  </Text>
                )}
              </TouchableOpacity>
            </View>
          </View>
        </View>
      </Modal>
    </SafeAreaView>
  );
}
