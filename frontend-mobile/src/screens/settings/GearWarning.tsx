// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The line a pair shows once its distance reaches the warning the athlete set — under it in Settings and on an activity run in it
// ABOUTME: Descriptive only: the pair's distance against the athlete's own threshold, never a claim about their body

import React from 'react';
import { Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import type { GearItem } from '@pierre/shared-types';
import { formatGearDistance, useDistanceUnit } from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';

/**
 * "This pair has 512 km, past the 500 km you set." on the warning tint, the
 * text in the ink that tint binds — in miles for an athlete who reads them.
 */
export function GearWarning({ pair }: { pair: GearItem }) {
  const { t, language } = useTranslation();
  const unit = useDistanceUnit();
  const colors = useThemeColors();
  return (
    <View
      className="mt-2 flex-row items-start gap-2 rounded-lg bg-warning/10 px-3 py-2"
      testID={`gear-warning-${pair.id}`}
    >
      <View className="mt-0.5">
        <Feather name="alert-triangle" size={16} color={colors.ink.warning} />
      </View>
      <Text className="flex-1 text-sm text-on-warning-container">
        {t('gear.warnReached', {
          distance: formatGearDistance(pair.distance_meters, unit, language),
          warn: formatGearDistance(pair.warn_distance_meters, unit, language),
        })}
      </Text>
    </View>
  );
}
