// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The check-in choice in Settings — every morning, three a week, weekly or never — the onboarding answer, changeable
// ABOUTME: Writes the server-side cadence the coach's check-in worker reads (carnet#831)

import { useTranslation } from '@pierre/i18n';
import { isCheckInCadence } from '@pierre/shared-types';
import { CHECK_IN_OPTIONS } from '@pierre/ui-logic';
import { Select } from '../ui';
import { useCheckIn } from '../../hooks/useCheckIn';

/**
 * How often the coach writes first. An athlete who never chose is shown
 * `off`, which is what they get: no check-in until they pick one.
 */
export default function CheckInSettings() {
  const { t } = useTranslation();
  const checkIn = useCheckIn();
  const preference = checkIn.preference;

  return (
    <div className="mt-5 pt-5 border-t ghost-border-faint flex items-start justify-between gap-4" data-testid="check-in-settings">
      <div className="min-w-0">
        <p className="font-medium text-on-surface">{t('settings.checkIn')}</p>
        <p className="text-sm text-on-surface-variant">{t('settings.checkInDescription')}</p>
        {checkIn.saveFailed && (
          <p role="alert" className="mt-1 text-xs text-error">
            {t('settings.checkInSaveFailed')}
          </p>
        )}
      </div>
      <div className="inline-block w-56 shrink-0">
        <Select
          data-testid="check-in-settings-select"
          value={preference?.cadence ?? 'off'}
          onChange={(e) => {
            if (isCheckInCadence(e.target.value)) checkIn.update(e.target.value);
          }}
          aria-label={t('settings.selectCheckIn')}
          disabled={preference === null || checkIn.isUpdating}
          options={CHECK_IN_OPTIONS.map((option) => ({ value: option.value, label: t(option.labelKey) }))}
        />
      </div>
    </div>
  );
}
