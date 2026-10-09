// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The units choice in Settings — Automatic, Metric or Imperial — and what Automatic resolved to
// ABOUTME: Writes the server-side preference the web, the phone and the agent's replies all read (carnet#835)

import { useTranslation } from '@pierre/i18n';
import type { UnitPreference } from '@pierre/shared-types';
import { UNIT_PREFERENCE_OPTIONS, unitsAutomaticHint } from '@pierre/ui-logic';
import { Select } from '../ui';
import { useUnitPreferences } from '../../hooks/useUnits';

/**
 * The athlete's units: their explicit choice always wins; Automatic follows
 * the connected provider's own setting, else the device's region.
 */
export default function UnitsSettings() {
  const { t } = useTranslation();
  const units = useUnitPreferences();
  const prefs = units.preferences;
  const hint = prefs === null ? null : unitsAutomaticHint(prefs);

  return (
    <div className="mt-5 pt-5 border-t ghost-border-faint flex items-start justify-between gap-4" data-testid="units-settings">
      <div className="min-w-0">
        <p className="font-medium text-on-surface">{t('settings.units')}</p>
        <p className="text-sm text-on-surface-variant">{t('settings.unitsDescription')}</p>
        {hint !== null && (
          <p className="mt-1 text-xs text-on-surface-variant" data-testid="units-settings-hint">
            {t(hint.key, { provider: hint.provider, system: t(hint.systemKey) })}
          </p>
        )}
        {units.saveFailed && (
          <p role="alert" className="mt-1 text-xs text-error">
            {t('settings.unitsSaveFailed')}
          </p>
        )}
      </div>
      <div className="inline-block w-44 shrink-0">
        <Select
          data-testid="units-settings-select"
          value={prefs?.preference ?? 'automatic'}
          onChange={(e) => units.update(e.target.value as UnitPreference)}
          aria-label={t('settings.selectUnits')}
          disabled={prefs === null || units.isUpdating}
          options={UNIT_PREFERENCE_OPTIONS.map((option) => ({ value: option.value, label: t(option.labelKey) }))}
        />
      </div>
    </div>
  );
}
