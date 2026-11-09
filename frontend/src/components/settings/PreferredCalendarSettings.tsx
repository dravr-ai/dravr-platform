// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The calendar choice in Connections settings — which connected calendar Dravr's plan and workouts go on
// ABOUTME: Shown with several calendars or a recorded one; the agent asks when none is chosen (carnet#1393)

import { useTranslation } from '@pierre/i18n';
import { Select } from '../ui';
import { usePreferredCalendar } from '../../hooks/usePreferredCalendar';

interface PreferredCalendarSettingsProps {
  /**
   * The connected providers as the cards above show them, in any stable
   * form: when it changes, a connection was made or removed here, and the
   * calendars to choose from are read again.
   */
  connected: string;
}

/** The connected calendar Dravr writes to, or none chosen, in which case the agent asks. */
export default function PreferredCalendarSettings({ connected }: PreferredCalendarSettingsProps) {
  const { t } = useTranslation();
  const preferred = usePreferredCalendar(connected);

  if (!preferred.visible || preferred.calendar === null) return null;
  const { chosen, chosen_connected: chosenConnected, calendars } = preferred.calendar;

  return (
    <div
      className="mt-5 pt-5 border-t ghost-border-faint flex items-start justify-between gap-4"
      data-testid="preferred-calendar-settings"
    >
      <div className="min-w-0">
        <p className="font-medium text-on-surface">{t('settings.preferredCalendar')}</p>
        <p className="text-sm text-on-surface-variant">{t('settings.preferredCalendarDescription')}</p>
        {chosen !== null && !chosenConnected && (
          <p className="mt-1 text-xs text-on-surface-variant" data-testid="preferred-calendar-disconnected">
            {t('settings.preferredCalendarDisconnected', { name: chosen.name })}
          </p>
        )}
        {preferred.saveFailed && (
          <p role="alert" className="mt-1 text-xs text-error">
            {t('settings.preferredCalendarSaveFailed')}
          </p>
        )}
      </div>
      <div className="inline-block w-44 shrink-0">
        <Select
          data-testid="preferred-calendar-select"
          value={preferred.selected ?? ''}
          onChange={(e) => preferred.choose(e.target.value === '' ? null : e.target.value)}
          aria-label={t('settings.preferredCalendar')}
          disabled={preferred.isUpdating}
          options={[
            { value: '', label: t('settings.preferredCalendarNone') },
            ...calendars.map((calendar) => ({ value: calendar.provider, label: calendar.name })),
          ]}
        />
      </div>
    </div>
  );
}
