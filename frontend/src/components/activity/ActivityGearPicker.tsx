// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The pair of shoes an activity's view says it was run in, and the athlete's choice of another pair or of none
// ABOUTME: Shown on a run, walk or hike only; a pair given by default says so, and a pair past its warning shows it here too

import { useTranslation } from '@pierre/i18n';
import { settingsPaneWebRoute } from '@pierre/shared-constants';
import { describeApiError, gearChoices } from '@pierre/ui-logic';
import { Button, Select } from '../ui';
import { GearWarning } from '../settings/GearWarning';
import { useActivityGear, useGearLedger } from '../../hooks/useGear';
import type { ActivityRef } from './activityRoute';

/** The picker's value for "none of my pairs": a pair id is a UUID, so it cannot collide. */
const NONE = 'none';

interface ActivityGearPickerProps {
  /** The activity as the view was opened on it: the choice is recorded on this copy. */
  activity: ActivityRef;
  /** Dashboard route navigator, `tab[/subview]`: where "Add shoes" goes. */
  onNavigate: (route: string) => void;
}

export function ActivityGearPicker({ activity, onNavigate }: ActivityGearPickerProps) {
  const { t } = useTranslation();
  const ledger = useGearLedger();
  const gear = useActivityGear(activity.provider, activity.id, true);
  const attribution = gear.attribution;

  if (ledger.isPending || gear.isLoading) {
    return (
      <div className="flex py-1" role="status" aria-label={t('common.loading')} data-testid="activity-gear">
        <div className="pierre-spinner h-5 w-5" />
      </div>
    );
  }
  if (ledger.isError || gear.isError || attribution === undefined) {
    return (
      <p className="text-sm text-on-surface-variant" data-testid="activity-gear">
        {describeApiError(ledger.error ?? gear.error, { t, fallbackKey: 'gear.loadFailed' })}
      </p>
    );
  }

  const pairs = ledger.data.gear;
  const choices = gearChoices(pairs, attribution.gear_id);
  if (choices.length === 0 && attribution.gear_id === null) {
    return (
      <div className="space-y-1" data-testid="activity-gear">
        <p className="text-sm text-on-surface-variant">{t('gear.activity.noPairs')}</p>
        <Button
          type="button"
          variant="tertiary"
          size="sm"
          onClick={() => onNavigate(settingsPaneWebRoute('gear'))}
          data-testid="activity-gear-add"
        >
          {t('gear.activity.addPairs')}
        </Button>
      </div>
    );
  }

  let value = '';
  if (attribution.gear_id !== null) {
    value = attribution.gear_id;
  } else if (attribution.source === 'declared') {
    value = NONE;
  }
  const chosen = pairs.find((pair) => pair.id === attribution.gear_id);
  const options = [
    ...choices.map((pair) =>
      pair.retired_at === null
        ? { value: pair.id, label: pair.name }
        : { value: pair.id, label: t('gear.activity.retiredOption', { name: pair.name }), disabled: true },
    ),
    { value: NONE, label: t('gear.activity.none') },
  ];

  return (
    <div data-testid="activity-gear">
      <Select
        label={t('gear.activity.label')}
        value={value}
        placeholder={value === '' ? t('gear.activity.choose') : undefined}
        options={options}
        disabled={gear.isChoosing}
        onChange={(event) => gear.choose(event.target.value === NONE ? null : event.target.value)}
        helpText={attribution.source === 'default' ? t('gear.activity.default') : undefined}
        error={
          gear.chooseFailed
            ? describeApiError(gear.chooseError, { t, fallbackKey: 'gear.activity.saveFailed' })
            : undefined
        }
        data-testid="activity-gear-select"
      />
      {chosen?.warn_reached && <GearWarning pair={chosen} />}
    </div>
  );
}
