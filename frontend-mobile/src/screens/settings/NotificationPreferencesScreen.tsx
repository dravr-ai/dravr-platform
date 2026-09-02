// ABOUTME: Notification preferences screen — per-category mute, quiet hours and daily cap
// ABOUTME: Mobile half of the surface pair; web renders the same rows from the same hook

import React, { useMemo, useState } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  Switch,
  ActivityIndicator,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import type { NotificationCategory } from '@pierre/shared-types';
import {
  NOTIFICATION_CATEGORY_META,
  NOTIFICATION_MAX_PER_DAY_CHOICES,
  mergeNotificationPreferences,
  notificationPreferenceUpdate,
} from '../../../../packages/shared-constants/src/notifications';
import { spacing, useThemeColors } from '../../constants/theme';
import { useNotificationPreferences } from '../../hooks/useNotifications';
import { useTranslation } from '@pierre/i18n';

/**
 * What each category actually sends, in the athlete's words.
 *
 * Keys, not copy, and identical to the web tab's map: the two surfaces describe
 * the same switch, so a difference here would be a difference in what the
 * athlete believes muting costs them. Holding the English sentences inline was
 * that difference — the default locale is French, so this screen described every
 * category in a language the rest of the screen was not speaking.
 */
const CATEGORY_BLURB_KEYS: Record<NotificationCategory, string> = {
  training: 'notifPrefs.blurbTraining',
  recovery: 'notifPrefs.blurbRecovery',
  coach: 'notifPrefs.blurbCoach',
  achievement: 'notifPrefs.blurbAchievement',
  system: 'notifPrefs.blurbSystem',
  ai: 'notifPrefs.blurbAi',
  reminders: 'notifPrefs.blurbReminders',
};

/** Quiet-hours boundaries on the hour, plus "Off" as an empty value. */
const QUIET_HOUR_VALUES: readonly string[] = [
  '',
  ...Array.from({ length: 24 }, (_, hour) => `${String(hour).padStart(2, '0')}:00`),
];

/** The device's IANA zone, used when a category has never had one stored. */
function localTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

/**
 * Label for one daily-cap choice.
 *
 * Takes `t` rather than reading a module-level instance: the caller is inside
 * the component, so the label re-renders when the athlete changes language.
 */
function capLabel(choice: number | null, t: (key: string, opts?: Record<string, unknown>) => string): string {
  if (choice === null) return t('frag.noLimit');
  return choice === 1 ? t('frag.perDayOne') : t('frag.perDayN', { count: choice });
}

/**
 * Manage which notification categories reach this athlete.
 *
 * Rows come from `GET /api/notifications/preferences` merged over the shared
 * defaults — the endpoint returns overrides, not one row per category — and
 * every change goes back through `notificationPreferenceUpdate`, which
 * restates the whole row —
 * the endpoint is an upsert, so a partial request would erase the fields it
 * left out.
 */
export function NotificationPreferencesScreen() {
  const { t } = useTranslation();
  const router = useRouter();
  const colors = useThemeColors();
  const { preferences, isLoading, isError, updatePreference, isUpdating } =
    useNotificationPreferences();
  const [expanded, setExpanded] = useState<NotificationCategory | null>(null);

  // The shared merge: every category in the shared display order, each one
  // showing its stored override or the default it runs on until the athlete
  // changes it. Web calls the same function, so an account with nothing stored
  // sees the same seven rows on both.
  const rows = useMemo(() => mergeNotificationPreferences(preferences), [preferences]);

  const cardStyle: ViewStyle = {
    backgroundColor: colors.background.tertiary,
    borderWidth: 1,
    borderColor: colors.border.default,
    borderRadius: 16,
    overflow: 'hidden',
  };

  const chipStyle = (selected: boolean): ViewStyle => ({
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderRadius: 999,
    marginRight: 8,
    borderWidth: 1,
    borderColor: selected ? colors.pierre.violet : colors.border.default,
    backgroundColor: selected ? `${colors.pierre.violet}20` : 'transparent',
  });

  return (
    <SafeAreaView
      style={{ flex: 1, backgroundColor: colors.background.primary }}
      edges={['top']}
      testID="notification-preferences-screen"
    >
      <View
        style={{
          flexDirection: 'row',
          alignItems: 'center',
          paddingHorizontal: spacing.md,
          paddingVertical: spacing.sm,
        }}
      >
        <TouchableOpacity
          onPress={() => router.back()}
          testID="back-button"
          style={{ padding: 8, marginRight: 8 }}
        >
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text style={{ fontSize: 20, fontWeight: '600', color: colors.text.primary }}>
          {t('notifPrefs.title')}
        </Text>
      </View>

      {isLoading ? (
        <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center' }}>
          <ActivityIndicator size="large" color={colors.pierre.violet} testID="notification-prefs-loading" />
        </View>
      ) : isError ? (
        <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center', padding: spacing.lg }}>
          <Text
            style={{ color: colors.text.secondary, textAlign: 'center' }}
            testID="notification-prefs-error"
          >
            {t('notifPrefs.loadFailedMobile')}
          </Text>
        </View>
      ) : (
        <ScrollView contentContainerStyle={{ padding: spacing.md, gap: spacing.md }}>
          <Text style={{ fontSize: 14, color: colors.text.tertiary, lineHeight: 20 }}>
            {t('notifPrefs.intro')}
          </Text>

          <View style={cardStyle} testID="notification-prefs-list">
            {rows.map((pref, index) => {
              const meta = NOTIFICATION_CATEGORY_META[pref.category];
              const isOpen = expanded === pref.category;
              return (
                <View
                  key={pref.category}
                  style={
                    index < rows.length - 1
                      ? { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }
                      : undefined
                  }
                  testID={`notification-pref-${pref.category}`}
                >
                  <View
                    style={{
                      flexDirection: 'row',
                      alignItems: 'center',
                      paddingHorizontal: 16,
                      paddingVertical: 14,
                    }}
                  >
                    <View
                      style={{
                        width: 10,
                        height: 10,
                        borderRadius: 5,
                        marginRight: 12,
                        backgroundColor: meta?.color ?? colors.text.tertiary,
                      }}
                    />
                    <View style={{ flex: 1, marginRight: 12 }}>
                      <Text style={{ fontSize: 16, color: colors.text.primary }}>
                        {meta ? t(meta.labelKey) : pref.category}
                      </Text>
                      <Text style={{ fontSize: 13, color: colors.text.tertiary, marginTop: 2 }}>
                        {CATEGORY_BLURB_KEYS[pref.category]
                          ? t(CATEGORY_BLURB_KEYS[pref.category])
                          : t('notifPrefs.categoryBlurbFallback')}
                      </Text>
                    </View>
                    <Switch
                      testID={`notification-pref-switch-${pref.category}`}
                      value={pref.enabled}
                      disabled={isUpdating}
                      onValueChange={(next) =>
                        updatePreference(notificationPreferenceUpdate(pref, { enabled: next }))
                      }
                      trackColor={{
                        false: colors.background.secondary,
                        true: `${colors.pierre.violet}60`,
                      }}
                      thumbColor={pref.enabled ? colors.pierre.violet : colors.text.tertiary}
                    />
                  </View>

                  {pref.enabled && (
                    <TouchableOpacity
                      onPress={() => setExpanded(isOpen ? null : pref.category)}
                      testID={`notification-pref-details-${pref.category}`}
                      style={{ paddingHorizontal: 16, paddingBottom: 12 }}
                    >
                      <Text style={{ fontSize: 13, color: colors.pierre.violet }}>
                        {isOpen ? t('notifPrefs.hideQuietHours') : t('notifPrefs.quietHoursAndLimit')}
                      </Text>
                    </TouchableOpacity>
                  )}

                  {pref.enabled && isOpen && (
                    <View style={{ paddingHorizontal: 16, paddingBottom: 16, gap: 12 }}>
                      <View>
                        <Text style={{
                            fontSize: 12,
                            color: colors.text.tertiary,
                            marginBottom: 6,
                            // The literals here were typed in capitals; the
                            // corpus strings are sentence case, so the capitals
                            // move to the style where they belong. Uppercasing
                            // the string instead would have shouted in five
                            // languages whose rules for it are not English's.
                            textTransform: 'uppercase',
                          }}>
                          {t('notifPrefs.maxPerDay')}
                        </Text>
                        <ScrollView horizontal showsHorizontalScrollIndicator={false}>
                          {NOTIFICATION_MAX_PER_DAY_CHOICES.map((choice) => (
                            <TouchableOpacity
                              key={choice === null ? 'none' : choice}
                              testID={`notification-pref-cap-${pref.category}-${choice === null ? 'none' : choice}`}
                              style={chipStyle(pref.max_per_day === choice)}
                              onPress={() =>
                                updatePreference(
                                  notificationPreferenceUpdate(pref, {
                                    max_per_day: choice === null ? undefined : choice,
                                  }),
                                )
                              }
                            >
                              <Text
                                style={{
                                  fontSize: 13,
                                  color:
                                    pref.max_per_day === choice
                                      ? colors.pierre.violet
                                      : colors.text.secondary,
                                }}
                              >
                                {capLabel(choice, t)}
                              </Text>
                            </TouchableOpacity>
                          ))}
                        </ScrollView>
                      </View>

                      <View>
                        <Text style={{
                            fontSize: 12,
                            color: colors.text.tertiary,
                            marginBottom: 6,
                            // The literals here were typed in capitals; the
                            // corpus strings are sentence case, so the capitals
                            // move to the style where they belong. Uppercasing
                            // the string instead would have shouted in five
                            // languages whose rules for it are not English's.
                            textTransform: 'uppercase',
                          }}>
                          {t('notifPrefs.quietFrom')}
                        </Text>
                        <ScrollView horizontal showsHorizontalScrollIndicator={false}>
                          {QUIET_HOUR_VALUES.map((value) => (
                            <TouchableOpacity
                              key={value === '' ? 'off' : value}
                              testID={`notification-pref-quiet-start-${pref.category}-${value === '' ? 'off' : value}`}
                              style={chipStyle((pref.quiet_hours_start ?? '') === value)}
                              onPress={() =>
                                updatePreference(
                                  notificationPreferenceUpdate(pref, {
                                    quiet_hours_start: value === '' ? undefined : value,
                                    timezone: pref.timezone ?? localTimezone(),
                                  }),
                                )
                              }
                            >
                              <Text
                                style={{
                                  fontSize: 13,
                                  color:
                                    (pref.quiet_hours_start ?? '') === value
                                      ? colors.pierre.violet
                                      : colors.text.secondary,
                                }}
                              >
                                {value === '' ? t('notifPrefs.off') : value}
                              </Text>
                            </TouchableOpacity>
                          ))}
                        </ScrollView>
                      </View>

                      <View>
                        <Text style={{
                            fontSize: 12,
                            color: colors.text.tertiary,
                            marginBottom: 6,
                            // The literals here were typed in capitals; the
                            // corpus strings are sentence case, so the capitals
                            // move to the style where they belong. Uppercasing
                            // the string instead would have shouted in five
                            // languages whose rules for it are not English's.
                            textTransform: 'uppercase',
                          }}>
                          {t('notifPrefs.quietUntil')}
                        </Text>
                        <ScrollView horizontal showsHorizontalScrollIndicator={false}>
                          {QUIET_HOUR_VALUES.map((value) => (
                            <TouchableOpacity
                              key={value === '' ? 'off' : value}
                              testID={`notification-pref-quiet-end-${pref.category}-${value === '' ? 'off' : value}`}
                              style={chipStyle((pref.quiet_hours_end ?? '') === value)}
                              onPress={() =>
                                updatePreference(
                                  notificationPreferenceUpdate(pref, {
                                    quiet_hours_end: value === '' ? undefined : value,
                                    timezone: pref.timezone ?? localTimezone(),
                                  }),
                                )
                              }
                            >
                              <Text
                                style={{
                                  fontSize: 13,
                                  color:
                                    (pref.quiet_hours_end ?? '') === value
                                      ? colors.pierre.violet
                                      : colors.text.secondary,
                                }}
                              >
                                {value === '' ? t('notifPrefs.off') : value}
                              </Text>
                            </TouchableOpacity>
                          ))}
                        </ScrollView>
                      </View>
                    </View>
                  )}
                </View>
              );
            })}
          </View>
        </ScrollView>
      )}
    </SafeAreaView>
  );
}
