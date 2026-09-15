// ABOUTME: Notification preferences screen — per-category mute, quiet hours and daily cap
// ABOUTME: Mobile half of the surface pair; web renders the same rows from the same hook

import React, { useMemo, useState } from 'react';
import { View, Text, Pressable, Switch, ActivityIndicator } from 'react-native';
import { PaneScrollView, Section, TextTabs } from '../../components/ui';
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
  coach: 'notifPrefs.blurbAgent',
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

/** The tab key of a quiet-hours boundary: the hour itself, or `off` for none. */
const QUIET_OFF_KEY = 'off';

/** The tab key of the daily cap that means no cap. */
const CAP_NONE_KEY = 'none';

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
 *
 * Each category is a `Section` of its own: the label and blurb are the header,
 * the switch is the header's action, and the quiet-hours pickers are the
 * content. Sections separate themselves by the column's gap, so no card and no
 * divider is drawn between them (DESIGN.md §10).
 */
export function NotificationPreferencesScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { preferences, isLoading, isError, updatePreference, isUpdating } =
    useNotificationPreferences();
  const [expanded, setExpanded] = useState<NotificationCategory | null>(null);

  // The shared merge: every category in the shared display order, each one
  // showing its stored override or the default it runs on until the athlete
  // changes it. Web calls the same function, so an account with nothing stored
  // sees the same seven rows on both.
  const rows = useMemo(() => mergeNotificationPreferences(preferences), [preferences]);

  // The pickers' items are the same for every category, so they are built
  // once per language rather than once per open section.
  const capItems = useMemo(
    () =>
      NOTIFICATION_MAX_PER_DAY_CHOICES.map((choice) => ({
        key: choice === null ? CAP_NONE_KEY : String(choice),
        label: capLabel(choice, t),
      })),
    [t],
  );
  const quietHourItems = useMemo(
    () =>
      QUIET_HOUR_VALUES.map((value) => ({
        key: value === '' ? QUIET_OFF_KEY : value,
        label: value === '' ? t('notifPrefs.off') : value,
      })),
    [t],
  );

  return (
    <View className="flex-1 bg-background-primary" testID="notification-preferences-screen">
      {isLoading ? (
        <View className="flex-1 items-center justify-center">
          <ActivityIndicator size="large" color={colors.tokens.primary} testID="notification-prefs-loading" />
        </View>
      ) : isError ? (
        <Text className="text-sm text-error px-4 pt-6" testID="notification-prefs-error">
          {t('notifPrefs.loadFailedMobile')}
        </Text>
      ) : (
        <PaneScrollView
          contentContainerStyle={{ paddingTop: spacing.lg, paddingBottom: spacing.xl, gap: spacing.lg }}
        >
          <Text className="text-sm text-text-secondary px-4">{t('notifPrefs.intro')}</Text>

          <View className="gap-8" testID="notification-prefs-list">
            {rows.map((pref) => {
              const meta = NOTIFICATION_CATEGORY_META[pref.category];
              const isOpen = expanded === pref.category;
              const capValue =
                pref.max_per_day === null || pref.max_per_day === undefined
                  ? CAP_NONE_KEY
                  : String(pref.max_per_day);
              return (
                <Section
                  key={pref.category}
                  title={meta ? t(meta.labelKey) : pref.category}
                  description={
                    CATEGORY_BLURB_KEYS[pref.category]
                      ? t(CATEGORY_BLURB_KEYS[pref.category])
                      : t('notifPrefs.categoryBlurbFallback')
                  }
                  testID={`notification-pref-${pref.category}`}
                  actions={
                    <Switch
                      testID={`notification-pref-switch-${pref.category}`}
                      value={pref.enabled}
                      disabled={isUpdating}
                      onValueChange={(next) =>
                        updatePreference(notificationPreferenceUpdate(pref, { enabled: next }))
                      }
                      trackColor={{ false: colors.border.default, true: colors.tokens.primary }}
                    />
                  }
                >
                  {pref.enabled && (
                    // The link and the picker labels are plain lines and pay
                    // the pane's inset themselves; the tab rows run full-bleed
                    // and inset their own labels, so a long row scrolls to the
                    // pane's edge.
                    <View className="gap-3">
                      <Pressable
                        className="px-4"
                        onPress={() => setExpanded(isOpen ? null : pref.category)}
                        accessibilityRole="button"
                        accessibilityState={{ expanded: isOpen }}
                        testID={`notification-pref-details-${pref.category}`}
                      >
                        <Text className="text-sm font-medium text-primary">
                          {isOpen ? t('notifPrefs.hideQuietHours') : t('notifPrefs.quietHoursAndLimit')}
                        </Text>
                      </Pressable>

                      {isOpen && (
                        <View className="gap-3">
                          <View className="gap-1.5">
                            <Text className="text-sm text-text-secondary px-4">{t('notifPrefs.maxPerDay')}</Text>
                            <TextTabs
                              testID={`notification-pref-cap-${pref.category}`}
                              items={capItems}
                              value={capValue}
                              onChange={(key) =>
                                updatePreference(
                                  notificationPreferenceUpdate(pref, {
                                    max_per_day: key === CAP_NONE_KEY ? undefined : Number(key),
                                  }),
                                )
                              }
                            />
                          </View>

                          <View className="gap-1.5">
                            <Text className="text-sm text-text-secondary px-4">{t('notifPrefs.quietFrom')}</Text>
                            <TextTabs
                              testID={`notification-pref-quiet-start-${pref.category}`}
                              items={quietHourItems}
                              value={pref.quiet_hours_start ?? QUIET_OFF_KEY}
                              onChange={(key) =>
                                updatePreference(
                                  notificationPreferenceUpdate(pref, {
                                    quiet_hours_start: key === QUIET_OFF_KEY ? undefined : key,
                                    timezone: pref.timezone ?? localTimezone(),
                                  }),
                                )
                              }
                            />
                          </View>

                          <View className="gap-1.5">
                            <Text className="text-sm text-text-secondary px-4">{t('notifPrefs.quietUntil')}</Text>
                            <TextTabs
                              testID={`notification-pref-quiet-end-${pref.category}`}
                              items={quietHourItems}
                              value={pref.quiet_hours_end ?? QUIET_OFF_KEY}
                              onChange={(key) =>
                                updatePreference(
                                  notificationPreferenceUpdate(pref, {
                                    quiet_hours_end: key === QUIET_OFF_KEY ? undefined : key,
                                    timezone: pref.timezone ?? localTimezone(),
                                  }),
                                )
                              }
                            />
                          </View>
                        </View>
                      )}
                    </View>
                  )}
                </Section>
              );
            })}
          </View>
        </PaneScrollView>
      )}
    </View>
  );
}
