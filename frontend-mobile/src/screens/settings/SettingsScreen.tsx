// ABOUTME: The settings list — the athlete's profile header above one row per named pane
// ABOUTME: Rows come from the shared settings declaration, so web and the phone offer the same panes

import React from 'react';
import { View, Text, ScrollView, TouchableOpacity, type ViewStyle } from 'react-native';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import {
  ADMIN_HIDDEN_PANES,
  settingsPane,
  settingsPanesFor,
  type SettingsPane,
  type SettingsPaneId,
} from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';
import { InitialsAvatar } from '../../components/ui/InitialsAvatar';
import { useAuth } from '../../contexts/AuthContext';
import { useFeatureFlags, FEATURE_KEYS } from '../../hooks/useFeatureFlags';
import { BILLING_ENABLED } from '../../constants/features';

/** The Feather glyph each pane carries in the list. */
const PANE_ICONS: Record<SettingsPaneId, React.ComponentProps<typeof Feather>['name']> = {
  profile: 'user',
  connections: 'link',
  tokens: 'key',
  coaching: 'message-square',
  messaging: 'message-circle',
  notifications: 'bell',
  memory: 'cpu',
  privacy: 'shield',
  about: 'info',
  account: 'settings',
  billing: 'credit-card',
};

const rowStyle: ViewStyle = {
  flexDirection: 'row',
  alignItems: 'center',
  paddingVertical: 14,
  paddingHorizontal: 16,
};

/**
 * Settings, as a list of named destinations.
 *
 * This exists because the phone served the same settings as one scroll roughly
 * 1,200pt tall — privacy below the fold, help and legal below that — while web
 * served ten named panes, and the grouping between them drifted with nothing to
 * catch it. The rows are read from `SETTINGS_PANES`, the one declaration both
 * clients share, so a pane added on one surface cannot go missing on the other.
 */
export function SettingsScreen() {
  const router = useRouter();
  const { user } = useAuth();
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

  const cardStyle: ViewStyle = {
    backgroundColor: colors.background.tertiary,
    borderWidth: 1,
    borderColor: colors.border.default,
    borderRadius: 16,
    overflow: 'hidden',
  };

  const displayName = user?.display_name || user?.email?.split('@')[0] || t('app.athlete');

  const renderRow = (pane: SettingsPane, index: number) => (
    <TouchableOpacity
      key={pane.id}
      style={[
        rowStyle,
        index < panes.length - 1
          ? { borderBottomWidth: 1, borderBottomColor: colors.border.faint }
          : {},
      ]}
      onPress={() => router.push(pane.mobile as never)}
      testID={`settings-pane-${pane.id}`}
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
        <Feather name={PANE_ICONS[pane.id]} size={20} color={colors.text.secondary} />
      </View>
      <View style={{ flex: 1 }}>
        <Text className="text-base" style={{ color: colors.text.primary }}>{t(pane.nameKey)}</Text>
        <Text className="text-sm" style={{ color: colors.text.tertiary }}>{t(pane.hintKey)}</Text>
      </View>
      <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
    </TouchableOpacity>
  );

  return (
    <View style={{ flex: 1, backgroundColor: colors.background.primary }} testID="settings-screen">
      {/* The title is the native large title above; the header and the tab
          bar inset the scroll themselves. */}
      <ScrollView
        style={{ flex: 1 }}
        contentInsetAdjustmentBehavior="automatic"
        contentContainerStyle={{
          paddingBottom: spacing.lg,
          paddingHorizontal: spacing.md,
        }}
        showsVerticalScrollIndicator={false}
        testID="settings-scroll"
      >
        {/* The identity row: the same initials circle and colour hash the
            conversation list draws, the name and email beside it, and the way
            to the profile pane as an ink link — no hero ring, no filled pill. */}
        <View className="h-14 flex-row items-center my-2" testID="settings-profile-section">
          <InitialsAvatar
            initials={initialsFor(displayName)}
            slot={avatarSlot({ id: user?.id ?? displayName, coach_id: null, group_id: null })}
          />
          <View className="flex-1 ml-3">
            <Text className="text-base font-semibold text-text-primary" numberOfLines={1}>
              {displayName}
            </Text>
            <Text className="text-sm text-text-secondary" numberOfLines={1}>
              {user?.email}
            </Text>
          </View>
          <TouchableOpacity
            className="py-2 pl-3"
            hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
            onPress={() => router.push(settingsPane('profile').mobile as never)}
            testID="settings-edit-profile-button"
          >
            <Text className="text-sm font-medium text-primary">{t('app.editProfile')}</Text>
          </TouchableOpacity>
        </View>

        <View style={cardStyle} testID="settings-pane-list">
          {panes.map(renderRow)}
        </View>
      </ScrollView>
    </View>
  );
}
