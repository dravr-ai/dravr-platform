// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Notification preferences tab — per-category mute, quiet hours and daily cap
// ABOUTME: Web half of the surface pair; mobile renders the same rows from the same hook

import { useMemo, useState } from 'react';
import {
  NOTIFICATION_CATEGORIES,
  NOTIFICATION_CATEGORY_META,
  NOTIFICATION_MAX_PER_DAY_CHOICES,
  notificationPreferenceUpdate,
} from '@pierre/shared-constants';
import type { NotificationCategory, NotificationPreferenceItem } from '@pierre/shared-types';
import { Card, Select } from './ui';
import { useNotificationPreferences } from '../hooks/useNotifications';

/**
 * What each category actually sends, in the athlete's words.
 *
 * The shared metadata carries the label, the colour and the icon — the three
 * things a notification row needs. A preferences screen needs one more thing:
 * what muting the category costs you. That sentence is only meaningful next to
 * a switch, so it lives with the switch rather than in the shared metadata.
 */
const CATEGORY_BLURB: Record<NotificationCategory, string> = {
  training: 'Planned sessions, workout reminders and training-load changes.',
  recovery: 'Sleep, HRV and readiness alerts from your connected devices.',
  social: 'Group activity, invitations and comments from other athletes.',
  coach: 'Messages your coach sends you, including commitment check-ins.',
  achievement: 'Personal bests, streaks and milestones.',
  system: 'Account, connection and service notices.',
  ai: 'Proactive insights your coach surfaces between conversations.',
  reminders: 'Anything you asked to be reminded about.',
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
const QUIET_HOUR_OPTIONS = [
  { value: '', label: 'Off' },
  ...Array.from({ length: 24 }, (_, hour) => {
    const value = `${String(hour).padStart(2, '0')}:00`;
    return { value, label: value };
  }),
];

/** A `null` cap means "no limit"; the select round-trips that as an empty value. */
function capValue(max: number | null): string {
  return max === null ? '' : String(max);
}

/** Label for one daily-cap choice. */
function capLabel(choice: number | null): string {
  if (choice === null) return 'No limit';
  return choice === 1 ? '1 per day' : `${choice} per day`;
}

/**
 * Manage which notification categories reach this athlete.
 *
 * Every row is a category the server returned. A category the server does not
 * know about is not rendered, and a category it returns that this build has no
 * blurb for still renders with its shared label — the server's list is the list.
 */
export default function NotificationSettingsTab() {
  const { preferences, isLoading, isError, updatePreference, isUpdating } =
    useNotificationPreferences();
  const [expanded, setExpanded] = useState<NotificationCategory | null>(null);

  // Order by the shared display order so web and mobile list the categories the
  // same way, with anything the server added but the constant has not appended
  // rather than dropped.
  const rows = useMemo(() => {
    const byCategory = new Map<string, NotificationPreferenceItem>(
      preferences.map((p) => [p.category, p]),
    );
    const known = NOTIFICATION_CATEGORIES.map((c) => byCategory.get(c)).filter(
      (p): p is NotificationPreferenceItem => p !== undefined,
    );
    const extra = preferences.filter(
      (p) => !NOTIFICATION_CATEGORIES.includes(p.category),
    );
    return [...known, ...extra];
  }, [preferences]);

  if (isLoading) {
    return (
      <Card variant="dark">
        <p className="text-sm text-on-surface-variant" data-testid="notification-prefs-loading">
          Loading notification preferences…
        </p>
      </Card>
    );
  }

  if (isError) {
    return (
      <Card variant="dark">
        <p className="text-sm text-error" data-testid="notification-prefs-error">
          Could not load your notification preferences. Reload the page to try again.
        </p>
      </Card>
    );
  }

  return (
    <Card variant="dark">
      <h2 className="text-lg font-semibold text-on-surface mb-2">Notifications</h2>
      <p className="text-sm text-on-surface-variant mb-6">
        Choose what reaches you, when it may arrive, and how often. These settings apply to push
        notifications and to anything your coach sends you between conversations.
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
                      {meta?.label ?? pref.category}
                    </h3>
                  </div>
                  <p className="text-sm text-on-surface-variant leading-relaxed">
                    {CATEGORY_BLURB[pref.category] ?? 'Notifications in this category.'}
                  </p>
                </div>

                <button
                  type="button"
                  role="switch"
                  aria-checked={pref.enabled}
                  aria-label={`${meta?.label ?? pref.category} notifications`}
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
                  className="mt-3 text-xs font-label uppercase tracking-wide text-primary"
                  data-testid={`notification-pref-details-${pref.category}`}
                  aria-expanded={isOpen}
                  onClick={() => setExpanded(isOpen ? null : pref.category)}
                >
                  {isOpen ? 'Hide quiet hours & limit' : 'Quiet hours & limit'}
                </button>
              )}

              {pref.enabled && isOpen && (
                <div className="mt-4 grid grid-cols-1 sm:grid-cols-3 gap-4">
                  <Select
                    label="Max per day"
                    size="sm"
                    value={capValue(pref.max_per_day)}
                    data-testid={`notification-pref-cap-${pref.category}`}
                    options={NOTIFICATION_MAX_PER_DAY_CHOICES.map((choice) => ({
                      value: capValue(choice),
                      label: capLabel(choice),
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
                    label="Quiet from"
                    size="sm"
                    value={pref.quiet_hours_start ?? ''}
                    data-testid={`notification-pref-quiet-start-${pref.category}`}
                    options={QUIET_HOUR_OPTIONS}
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
                    label="Quiet until"
                    size="sm"
                    value={pref.quiet_hours_end ?? ''}
                    data-testid={`notification-pref-quiet-end-${pref.category}`}
                    options={QUIET_HOUR_OPTIONS}
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
