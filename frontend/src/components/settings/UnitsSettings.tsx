// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The units choice in Settings — Metric or Imperial; every athlete reads metric until they choose otherwise
// ABOUTME: Writes the server-side preference the web, the phone and the agent's replies all read (carnet#835, carnet#1044)

import { useTranslation } from '@pierre/i18n';
import type { UnitSystem } from '@pierre/shared-types';
import { UNIT_SYSTEM_OPTIONS } from '@pierre/ui-logic';
import { Select } from '../ui';
import { useUnitPreferences } from '../../hooks/useUnits';

/** The athlete's units: metric unless they choose imperial here. */
export default function UnitsSettings() {
  const { t } = useTranslation();
  const units = useUnitPreferences();

  return (
    <div className="mt-5 pt-5 border-t ghost-border-faint flex items-start justify-between gap-4" data-testid="units-settings">
      <div className="min-w-0">
        <p className="font-medium text-on-surface">{t('settings.units')}</p>
        <p className="text-sm text-on-surface-variant">{t('settings.unitsDescription')}</p>
        {units.saveFailed && (
          <p role="alert" className="mt-1 text-xs text-error">
            {t('settings.unitsSaveFailed')}
          </p>
        )}
      </div>
      <div className="inline-block w-44 shrink-0">
        <Select
          data-testid="units-settings-select"
          value={units.units ?? 'metric'}
          onChange={(e) => units.update(e.target.value as UnitSystem)}
          aria-label={t('settings.selectUnits')}
          disabled={units.units === null || units.isUpdating}
          options={UNIT_SYSTEM_OPTIONS.map((option) => ({ value: option.value, label: t(option.labelKey) }))}
        />
      </div>
    </div>
  );
}
