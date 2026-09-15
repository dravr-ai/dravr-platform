// ABOUTME: Account pane — status, usage, security, connected MCP apps and sign-out, as settings Sections of Rows
// ABOUTME: Section order comes from the shared settings declaration, so web groups the same five

import React, { useMemo, useState } from 'react';
import { ActivityIndicator, Alert, Pressable, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { settingsPaneSections } from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';
import { Button, EmptyState, Input, PaneScrollView, Row, Section, Sheet } from '../../components/ui';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useUsageStatus, type LimitCheckResult } from '../chat/useUsageStatus';
import { CONNECTED_APPS_ROUTE } from '../../navigation/routes';

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

/** A quota as the athlete reads it: what is used over what is allowed. */
function formatQuota(counter: LimitCheckResult, compact: boolean): string {
  const current = compact ? formatCompactNumber(counter.current) : counter.current.toLocaleString();
  const limit = compact ? formatCompactNumber(counter.limit) : counter.limit.toLocaleString();
  return `${current} / ${limit}`;
}

/**
 * Everything about the account itself.
 *
 * Status, usage, security and the connected MCP apps belong together, and web
 * has held them together since it had panes. The section order is read from the
 * shared declaration rather than typed twice, which is what let the phone
 * scatter the same four things down one scroll with nothing failing.
 *
 * Every group is a `Section` of `Row`s on the ground. A `Section` pays the
 * pane's 16 inset for its title and none for its content, and a `Row` pays the
 * same inset for itself so its hairline insets to the text while its press
 * target runs to the pane's edge; title and row text share one left edge.
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

  // Each figure is one fact row: the label on the left, `used / allowed` as
  // the mono value on the right. No meter — the number is the whole state.
  const usageRows = useMemo(() => {
    if (!usageData) return [];
    return [
      { label: t('app.dailyMessages'), value: formatQuota(usageData.daily.messages, false) },
      { label: t('app.dailyTokens'), value: formatQuota(usageData.daily.tokens, true) },
      { label: t('app.weeklyMessages'), value: formatQuota(usageData.weekly.messages, false) },
      { label: t('app.agents'), value: `${usageData.resources.agents} / ${usageData.resources.max_agents}` },
      {
        label: t('app.conversations'),
        value: `${usageData.resources.conversations} / ${usageData.resources.max_conversations}`,
      },
    ];
  }, [usageData, t]);

  const closeChangePassword = () => {
    setShowChangePassword(false);
    setCurrentPassword('');
    setNewPassword('');
    setConfirmPassword('');
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
      closeChangePassword();
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

  const renderSection = (section: string) => {
    switch (section) {
      case 'account-status':
        return (
          <Section key={section} title={t('profile.accountStatus')} testID="account-section-account-status">
            <Row compact title={t('settingsUi.status')} value={user?.user_status ?? t('settingsUi.unknownDate')} />
            <Row compact title={t('settingsUi.role')} value={user?.role ?? t('settingsUi.unknownDate')} />
            <Row
              compact
              last
              title={t('profile.memberSince')}
              value={formatMemberSince(user?.created_at, t('settingsUi.unknownDate'))}
            />
          </Section>
        );

      case 'usage':
        return (
          <Section key={section} title={t('app.usage')} testID="account-section-usage">
            {usageLoading ? (
              <View className="py-6 items-center">
                <ActivityIndicator size="small" color={colors.tokens.primary} />
              </View>
            ) : !usageData ? (
              <EmptyState>{t('app.usageDataUnavailable')}</EmptyState>
            ) : (
              <>
                {usageRows.map(({ label, value }, index) => (
                  <Row key={label} compact title={label} value={value} last={index === usageRows.length - 1} />
                ))}
                <Text className="text-xs text-text-tertiary px-4 mt-2">
                  {t('app.dailyLimitsResetAt', {
                    time: formatResetTime(usageData.daily.messages.resets_at, t('settingsUi.midnightUtc')),
                  })}
                </Text>
              </>
            )}
          </Section>
        );

      case 'security':
        return (
          <Section key={section} title={t('settingsUi.security')} testID="account-section-security">
            <Row
              last
              title={t('app.changePassword')}
              subtitle={t('password.changeHint')}
              onPress={() => setShowChangePassword(true)}
              testID="account-change-password-button"
            />
          </Section>
        );

      case 'connected-mcp-apps':
        return (
          <Section
            key={section}
            title={t('tokens.connectedMcpApps')}
            description={t('tokens.connectedAppsHint')}
            testID="account-section-connected-mcp-apps"
          >
            <Row
              last
              title={t('app.connectedApps')}
              onPress={() => router.push(CONNECTED_APPS_ROUTE)}
              testID="account-connected-apps-button"
            />
          </Section>
        );

      case 'sign-out':
        // Signing out is a quiet row in the secondary ink, the same one the
        // settings root ends with, and its one-line hint under it. There is no
        // group title: the row is its own name.
        return (
          <View key={section} testID="account-section-sign-out">
            <Pressable
              className="px-4 min-h-[52px] justify-center"
              onPress={handleLogout}
              accessibilityRole="button"
              testID="account-logout-button"
            >
              <Text className="text-sm text-text-secondary">{t('app.logOut')}</Text>
            </Pressable>
            <Text className="text-xs text-text-tertiary px-4">{t('account.signOutHint')}</Text>
          </View>
        );

      default:
        return null;
    }
  };

  return (
    <View style={{ flex: 1, backgroundColor: colors.background.primary }} testID="account-screen">
      <PaneScrollView contentContainerStyle={{ paddingVertical: spacing.md }}>
        <View className="gap-8">{settingsPaneSections('account').map(renderSection)}</View>
      </PaneScrollView>

      <Sheet visible={showChangePassword} onClose={closeChangePassword} testID="account-change-password-sheet">
        <Text className="text-xl font-semibold text-text-primary mb-5">{t('app.changePassword')}</Text>

        <Input
          label={t('app.currentPassword')}
          value={currentPassword}
          onChangeText={setCurrentPassword}
          secureTextEntry
          showPasswordToggle
          testID="account-current-password-input"
        />
        <Input
          label={t('app.newPassword')}
          value={newPassword}
          onChangeText={setNewPassword}
          secureTextEntry
          showPasswordToggle
          testID="account-new-password-input"
        />
        <Input
          label={t('app.confirmNewPassword')}
          value={confirmPassword}
          onChangeText={setConfirmPassword}
          secureTextEntry
          showPasswordToggle
          testID="account-confirm-password-input"
        />

        <View className="flex-row gap-3 mt-4">
          <Button
            title={t('common.cancel')}
            variant="ghost"
            onPress={closeChangePassword}
            style={{ flex: 1 }}
            testID="account-change-password-cancel"
          />
          <Button
            title={t('app.change')}
            variant="primary"
            onPress={() => { void handleChangePassword(); }}
            loading={isChangingPassword}
            style={{ flex: 1 }}
            testID="account-change-password-confirm"
          />
        </View>
      </Sheet>
    </View>
  );
}
