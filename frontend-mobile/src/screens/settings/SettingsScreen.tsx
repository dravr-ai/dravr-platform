// ABOUTME: The settings list — the athlete's profile header above one row per named pane
// ABOUTME: Rows come from the shared settings declaration, so web and the phone offer the same panes

import React from 'react';
import { View, Text, ScrollView, TouchableOpacity, type ViewStyle } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { LinearGradient } from 'expo-linear-gradient';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import {
  ADMIN_HIDDEN_PANES,
  settingsPane,
  settingsPanesFor,
  type SettingsPane,
  type SettingsPaneId,
} from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';
import { tabBarBottomOffset } from '../../components/ui/ExpandableTabBar';
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
  const insets = useSafeAreaInsets();
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
          ? { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }
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
        <Text style={{ fontSize: 16, color: colors.text.primary }}>{t(pane.nameKey)}</Text>
        <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t(pane.hintKey)}</Text>
      </View>
      <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
    </TouchableOpacity>
  );

  return (
    <View style={{ flex: 1, backgroundColor: colors.background.primary }} testID="settings-screen">
      {/* An opaque band the height of the status bar, outside the scroll, so
          nothing scrolls up behind the notch. A safe-area inset applied inside
          the scroll moves with the content and stops covering it. */}
      <View
        testID="settings-safe-header"
        style={{
          paddingTop: insets.top,
          paddingHorizontal: spacing.md,
          paddingBottom: spacing.sm,
          backgroundColor: colors.background.primary,
        }}
      >
        <Text style={{ fontSize: 22, fontWeight: '700', color: colors.text.primary }}>
          {t('common.settings')}
        </Text>
      </View>

      <ScrollView
        style={{ flex: 1 }}
        contentContainerStyle={{
          // The tab bar floats over the scroll, so the last row would otherwise
          // sit half-hidden behind it.
          paddingBottom: tabBarBottomOffset(insets.bottom),
          paddingHorizontal: spacing.md,
        }}
        showsVerticalScrollIndicator={false}
        testID="settings-scroll"
      >
        <View style={{ alignItems: 'center', paddingVertical: 24 }} testID="settings-profile-section">
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
            <View
              style={{
                width: '100%',
                height: '100%',
                borderRadius: 56,
                backgroundColor: colors.background.primary,
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              <Text style={{ fontSize: 36, fontWeight: 'bold', color: colors.text.primary }}>
                {displayName[0]?.toUpperCase() ?? '?'}
              </Text>
            </View>
          </LinearGradient>

          <Text style={{ fontSize: 24, fontWeight: 'bold', color: colors.text.primary, marginBottom: 4 }}>
            {displayName}
          </Text>
          <Text style={{ fontSize: 16, color: colors.text.tertiary, marginBottom: 16 }}>{user?.email}</Text>

          <TouchableOpacity
            style={{
              paddingHorizontal: 24,
              paddingVertical: 10,
              borderRadius: 9999,
              backgroundColor: colors.pierre.violet,
            }}
            onPress={() => router.push(settingsPane('profile').mobile as never)}
            testID="settings-edit-profile-button"
          >
            <Text style={{ fontSize: 14, fontWeight: '600', color: colors.tokens.onPrimary }}>
              {t('app.editProfile')}
            </Text>
          </TouchableOpacity>
        </View>

        <View style={cardStyle} testID="settings-pane-list">
          {panes.map(renderRow)}
        </View>
      </ScrollView>
    </View>
  );
}
