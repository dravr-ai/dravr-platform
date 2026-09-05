// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Notification preferences tab — per-category mute, quiet hours and daily cap
// ABOUTME: Web half of the surface pair; mobile renders the same rows from the same hook

import { useMemo, useState } from 'react';
import {
  NOTIFICATION_CATEGORY_META,
  NOTIFICATION_MAX_PER_DAY_CHOICES,
  mergeNotificationPreferences,
  notificationPreferenceUpdate,
} from '@pierre/shared-constants';
import type { NotificationCategory } from '@pierre/shared-types';
import { Card, Select } from './ui';
import { useNotificationPreferences } from '../hooks/useNotifications';
import { useTranslation } from '@pierre/i18n';

/**
 * What each category actually sends, in the athlete's words.
 *
 * The shared metadata carries the label, the colour and the icon — the three
 * things a notification row needs. A preferences screen needs one more thing:
 * what muting the category costs you. That sentence is only meaningful next to
 * a switch, so it lives with the switch rather than in the shared metadata.
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

/** The browser's IANA zone, used when a category has never had one stored. */
function localTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

/**
 * Quiet-hours boundaries on the hour, plus "Off".
 *
 * The wire format is `HH:MM`; the server compares against it directly, so the
 * options are generated in that format rather than parsed back from a label.
 */
function quietHourOptions(offLabel: string) {
  return [
  { value: '', label: offLabel },
  ...Array.from({ length: 24 }, (_, hour) => {
    const value = `${String(hour).padStart(2, '0')}:00`;
    return { value, label: value };
  }),
];
}

/** A `null` cap means "no limit"; the select round-trips that as an empty value. */
function capValue(max: number | null): string {
  return max === null ? '' : String(max);
}

/**
 * Label for one daily-cap choice.
 *
 * Takes the translator rather than calling the hook: this is a plain helper,
 * not a component, and a hook here is a rules-of-hooks violation.
 */
function capLabel(choice: number | null, t: (key: string, opts?: Record<string, unknown>) => string): string {
  if (choice === null) return t('frag.noLimit');
  return choice === 1 ? t('frag.perDayOne') : t('frag.perDayN', { count: choice });
}

/**
 * Manage which notification categories reach this athlete.
 *
 * Every category gets a row: the stored override where there is one, the
 * default the dispatcher already applies where there is not. A category the
 * server returns that this build has no blurb for still renders with its
 * shared label.
 */
export default function NotificationSettingsTab() {
  const { t } = useTranslation();
  const { preferences, isLoading, isError, updatePreference, isUpdating } =
    useNotificationPreferences();
  const [expanded, setExpanded] = useState<NotificationCategory | null>(null);

  // The shared merge: every category in the shared display order, each one
  // showing its stored override or the default it runs on until the athlete
  // changes it. Web and mobile call the same function so neither can invent a
  // different answer for an account with nothing stored.
  const rows = useMemo(() => mergeNotificationPreferences(preferences), [preferences]);

  if (isLoading) {
    return (
      <Card variant="dark">
        <p className="text-sm text-on-surface-variant" data-testid="notification-prefs-loading">
          {t('notifPrefs.loading')}
        </p>
      </Card>
    );
  }

  if (isError) {
    return (
      <Card variant="dark">
        <p className="text-sm text-error" data-testid="notification-prefs-error">
          {t('notifPrefs.loadFailed')}
        </p>
      </Card>
    );
  }

  return (
    <Card variant="dark">
      <h2 className="text-lg font-semibold text-on-surface mb-2">{t('notifPrefs.title')}</h2>
      <p className="text-sm text-on-surface-variant mb-6">
        {t('notifPrefs.intro')}
      </p>

      <div className="space-y-3" data-testid="notification-prefs-list">
        {rows.map((pref) => {
          const meta = NOTIFICATION_CATEGORY_META[pref.category];
          const isOpen = expanded === pref.category;
          return (
            <div
              key={pref.category}
              className="p-4 bg-surface-container-low rounded-xl border ghost-border"
              data-testid={`notification-pref-${pref.category}`}
            >
              <div className="flex items-start justify-between gap-4">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-3 mb-1">
                    <span
                      className="w-2.5 h-2.5 rounded-full flex-shrink-0"
                      style={{ backgroundColor: meta?.color ?? 'currentColor' }}
                      aria-hidden="true"
                    />
                    <h3 className="text-sm font-medium text-on-surface">
                      {meta ? t(meta.labelKey) : pref.category}
                    </h3>
                  </div>
                  <p className="text-sm text-on-surface-variant leading-relaxed">
                    {CATEGORY_BLURB_KEYS[pref.category] ? t(CATEGORY_BLURB_KEYS[pref.category]) : t('notifPrefs.categoryBlurbFallback')}
                  </p>
                </div>

                <button
                  type="button"
                  role="switch"
                  aria-checked={pref.enabled}
                  aria-label={t('notifPrefs.categoryNotifications', { category: meta ? t(meta.labelKey) : pref.category })}
                  data-testid={`notification-pref-switch-${pref.category}`}
                  disabled={isUpdating}
                  onClick={() =>
                    updatePreference(
                      notificationPreferenceUpdate(pref, { enabled: !pref.enabled }),
                    )
                  }
                  className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-primary disabled:opacity-50 ${
                    pref.enabled ? 'bg-primary' : 'bg-surface-container-high'
                  }`}
                >
                  <span
                    className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                      pref.enabled ? 'translate-x-5' : 'translate-x-0'
                    }`}
                  />
                </button>
              </div>

              {pref.enabled && (
                <button
                  type="button"
                  className="mt-3 text-xs text-primary"
                  data-testid={`notification-pref-details-${pref.category}`}
                  aria-expanded={isOpen}
                  onClick={() => setExpanded(isOpen ? null : pref.category)}
                >
                  {isOpen ? t('notifPrefs.hideQuietHours') : t('notifPrefs.quietHoursAndLimit')}
                </button>
              )}

              {pref.enabled && isOpen && (
                <div className="mt-4 grid grid-cols-1 sm:grid-cols-3 gap-4">
                  <Select
                    label={t('notifPrefs.maxPerDay')}
                    size="sm"
                    value={capValue(pref.max_per_day)}
                    data-testid={`notification-pref-cap-${pref.category}`}
                    options={NOTIFICATION_MAX_PER_DAY_CHOICES.map((choice) => ({
                      value: capValue(choice),
                      label: capLabel(choice, t),
                    }))}
                    onChange={(e) =>
                      updatePreference(
                        notificationPreferenceUpdate(pref, {
                          max_per_day: e.target.value === '' ? undefined : Number(e.target.value),
                        }),
                      )
                    }
                  />
                  <Select
                    label={t('notifPrefs.quietFrom')}
                    size="sm"
                    value={pref.quiet_hours_start ?? ''}
                    data-testid={`notification-pref-quiet-start-${pref.category}`}
                    options={quietHourOptions(t('notifPrefs.off'))}
                    onChange={(e) =>
                      updatePreference(
                        notificationPreferenceUpdate(pref, {
                          quiet_hours_start: e.target.value === '' ? undefined : e.target.value,
                          timezone: pref.timezone ?? localTimezone(),
                        }),
                      )
                    }
                  />
                  <Select
                    label={t('notifPrefs.quietUntil')}
                    size="sm"
                    value={pref.quiet_hours_end ?? ''}
                    data-testid={`notification-pref-quiet-end-${pref.category}`}
                    options={quietHourOptions(t('notifPrefs.off'))}
                    onChange={(e) =>
                      updatePreference(
                        notificationPreferenceUpdate(pref, {
                          quiet_hours_end: e.target.value === '' ? undefined : e.target.value,
                          timezone: pref.timezone ?? localTimezone(),
                        }),
                      )
                    }
                  />
                </div>
              )}
            </div>
          );
        })}
      </div>
    </Card>
  );
}
