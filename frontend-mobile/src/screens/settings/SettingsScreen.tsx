// ABOUTME: The settings list — the athlete's identity row above one settings Row per named pane, then the quiet sign-out
// ABOUTME: Rows come from the shared settings declaration, so web and the phone offer the same panes

import React from 'react';
import { Alert, Pressable, ScrollView, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { ADMIN_HIDDEN_PANES, settingsPane, settingsPanesFor } from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';
import { InitialsAvatar, Row } from '../../components/ui';
import { useAuth } from '../../contexts/AuthContext';
import { useFeatureFlags, FEATURE_KEYS } from '../../hooks/useFeatureFlags';
import { BILLING_ENABLED } from '../../constants/features';

/**
 * Settings, as a list of named destinations.
 *
 * This exists because the phone served the same settings as one scroll roughly
 * 1,200pt tall — privacy below the fold, help and legal below that — while web
 * served ten named panes, and the grouping between them drifted with nothing to
 * catch it. The rows are read from `SETTINGS_PANES`, the one declaration both
 * clients share, so a pane added on one surface cannot go missing on the other.
 *
 * Each pane is a plain settings `Row`: its name, the current state as the
 * inline hint, a chevron. No glyph square and no card — the rows sit on the
 * ground and the hairline between them is the only separator (DESIGN.md §10).
 */
export function SettingsScreen() {
  const router = useRouter();
  const { user, logout } = useAuth();
  const colors = useThemeColors();
  const { t } = useTranslation();
  const { flags: featureFlags } = useFeatureFlags();

  // Operators are platform staff: provider connections, messaging and About are
  // athlete-account surfaces, hidden from them exactly as on web. Gate on
  // `role` to stay consistent with the web Dashboard.
  const isAdminUser = user?.role === 'admin' || user?.role === 'super_admin';

  const panes = settingsPanesFor('mobile').filter((pane) => {
    if (isAdminUser && ADMIN_HIDDEN_PANES.has(pane.id)) return false;
    if (pane.flag === 'api_tokens') return Boolean(featureFlags[FEATURE_KEYS.apiTokens]);
    if (pane.flag === 'billing') return BILLING_ENABLED;
    return true;
  });

  const displayName = user?.display_name || user?.email?.split('@')[0] || t('app.athlete');

  // The same confirmation the Account pane asks for, so signing out reads the
  // same wherever the athlete reaches it from.
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

  return (
    <View style={{ flex: 1, backgroundColor: colors.background.primary }} testID="settings-screen">
      {/* The title is the native large title above; the header and the tab
          bar inset the scroll themselves. The rows pay their own 16 inset so
          their hairlines reach the pane's right edge. */}
      <ScrollView
        style={{ flex: 1 }}
        contentInsetAdjustmentBehavior="automatic"
        contentContainerStyle={{ paddingBottom: spacing.lg }}
        showsVerticalScrollIndicator={false}
        testID="settings-scroll"
      >
        {/* The identity row: the same initials circle and colour hash the
            conversation list draws, the name and email beside it, and the way
            to the profile pane as an ink link — no hero ring, no filled pill. */}
        <View className="h-14 flex-row items-center my-2 px-4" testID="settings-profile-section">
          <InitialsAvatar
            initials={initialsFor(displayName)}
            slot={avatarSlot({ id: user?.id ?? displayName, agent_id: null, group_id: null })}
            size={40}
          />
          <View className="flex-1 ml-3">
            <Text className="text-base font-semibold text-text-primary" numberOfLines={1}>
              {displayName}
            </Text>
            <Text className="text-sm text-text-secondary" numberOfLines={1}>
              {user?.email}
            </Text>
          </View>
          <Pressable
            className="py-2 pl-3"
            hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
            onPress={() => router.push(settingsPane('profile').mobile as never)}
            accessibilityRole="button"
            testID="settings-edit-profile-button"
          >
            <Text className="text-sm font-medium text-primary">{t('app.editProfile')}</Text>
          </Pressable>
        </View>

        <View testID="settings-pane-list">
          {panes.map((pane, index) => (
            <Row
              key={pane.id}
              title={t(pane.nameKey)}
              hint={t(pane.hintKey)}
              onPress={() => router.push(pane.mobile as never)}
              last={index === panes.length - 1}
              testID={`settings-pane-${pane.id}`}
            />
          ))}
        </View>

        {/* Signing out is a quiet row after the list, in the secondary ink
            with no chevron: it goes nowhere in the app, and it is not the
            thing this screen is for. */}
        <Pressable
          className="mt-8 px-4 min-h-[52px] justify-center"
          onPress={handleLogout}
          accessibilityRole="button"
          testID="settings-sign-out"
        >
          <Text className="text-sm text-text-secondary">{t('app.logOut')}</Text>
        </Pressable>
      </ScrollView>
    </View>
  );
}
