// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Notification preferences tab — per-category mute, quiet hours and daily cap
// ABOUTME: Web half of the surface pair; mobile renders the same rows from the same hook

import { useMemo, useState } from 'react';
import {
  NOTIFICATION_CATEGORY_BLURB_KEYS,
  NOTIFICATION_CATEGORY_COLORS,
  NOTIFICATION_CATEGORY_META,
  NOTIFICATION_MAX_PER_DAY_CHOICES,
  NOTIFICATION_QUIET_HOUR_VALUES,
  localTimezone,
  mergeNotificationPreferences,
  notificationCapLabel,
  notificationPreferenceUpdate,
} from '@pierre/shared-constants';
import type { NotificationCategory } from '@pierre/shared-types';
import { Section, Select } from './ui';
import { useNotificationPreferences } from '../hooks/useNotifications';
import { useTheme } from '../hooks/useTheme';
import { useTranslation } from '@pierre/i18n';

/**
 * Quiet-hours boundaries as select options, the empty value labelled "Off".
 *
 * The values are the shared `HH:MM` list the server compares against directly,
 * so an option is never parsed back from its label.
 */
function quietHourOptions(offLabel: string) {
  return NOTIFICATION_QUIET_HOUR_VALUES.map((value) => ({
    value,
    label: value === '' ? offLabel : value,
  }));
}

/** A `null` cap means "no limit"; the select round-trips that as an empty value. */
function capValue(max: number | null): string {
  return max === null ? '' : String(max);
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
  // The dot takes the hue paired with the athlete's scheme: the map is total,
  // so a category never falls back to the text ink.
  const { scheme } = useTheme();
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
      <p className="text-sm text-on-surface-variant" data-testid="notification-prefs-loading">
        {t('notifPrefs.loading')}
      </p>
    );
  }

  if (isError) {
    return (
      <p className="text-sm text-error" data-testid="notification-prefs-error">
        {t('notifPrefs.loadFailed')}
      </p>
    );
  }

  return (
    <Section title={t('notifPrefs.title')} description={t('notifPrefs.intro')}>
      <div data-testid="notification-prefs-list">
        {rows.map((pref) => {
          const meta = NOTIFICATION_CATEGORY_META[pref.category];
          const isOpen = expanded === pref.category;
          return (
            <div
              key={pref.category}
              className="border-t ghost-border-faint py-3 first:border-t-0"
              data-testid={`notification-pref-${pref.category}`}
            >
              <div className="flex items-start justify-between gap-4">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-3 mb-1">
                    <span
                      className="w-2.5 h-2.5 rounded-full flex-shrink-0"
                      style={{ backgroundColor: NOTIFICATION_CATEGORY_COLORS[scheme][pref.category] }}
                      aria-hidden="true"
                    />
                    <h3 className="text-sm font-medium text-on-surface">
                      {meta ? t(meta.labelKey) : pref.category}
                    </h3>
                  </div>
                  <p className="text-sm text-on-surface-variant leading-relaxed">
                    {NOTIFICATION_CATEGORY_BLURB_KEYS[pref.category] ? t(NOTIFICATION_CATEGORY_BLURB_KEYS[pref.category]) : t('notifPrefs.categoryBlurbFallback')}
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
                      label: notificationCapLabel(choice, t),
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
    </Section>
  );
}
