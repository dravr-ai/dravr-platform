// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The line a pair shows once its distance reaches the warning the athlete set — in Settings and on an activity run in it
// ABOUTME: Descriptive only: the pair's distance against the athlete's own threshold, never a claim about their body

import { AlertTriangle } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { GearItem } from '@pierre/shared-types';
import { formatGearDistance, useDistanceUnit } from '@pierre/ui-logic';

/**
 * "This pair has 512 km, past the 500 km you set." on the warning tint, with
 * its icon — in miles for an athlete who reads them.
 */
export function GearWarning({ pair }: { pair: GearItem }) {
  const { t, language } = useTranslation();
  const unit = useDistanceUnit();
  return (
    <p
      className="mt-2 flex items-start gap-2 rounded-lg bg-warning/10 px-3 py-2 text-sm text-on-warning-container"
      data-testid={`gear-warning-${pair.id}`}
    >
      <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
      <span>
        {t('gear.warnReached', {
          distance: formatGearDistance(pair.distance_meters, unit, language),
          warn: formatGearDistance(pair.warn_distance_meters, unit, language),
        })}
      </span>
    </p>
  );
}
