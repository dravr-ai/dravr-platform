// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The pair of shoes an activity's screen says it was run in, and the athlete's choice of another pair or of none
// ABOUTME: A settings Row opening a Sheet of the pairs in use; a pair given by default says so, and one past its warning shows it here too

import React, { useState } from 'react';
import { ActivityIndicator, ScrollView, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { describeApiError, formatGearDistance, gearChoices, useDistanceUnit } from '@pierre/ui-logic';
import { EmptyState, Row, Sheet } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useActivityGear, useGearLedger } from '../../hooks/useGear';
import { GEAR_ROUTE } from '../../navigation/routes';
import { GearWarning } from '../settings/GearWarning';
import { sportLabel } from '../home/homeFormat';

interface ActivityGearPickerProps {
  /** The activity as the screen was opened on it: the choice is recorded on this copy. */
  provider: string;
  activityId: string;
}

/** One line of the sheet: a pair in use, the retired pair the activity counts on, or none of them. */
interface GearOption {
  key: string;
  /** The pair's id, `null` for none of the athlete's pairs. */
  gearId: string | null;
  label: string;
  subtitle?: string;
  value?: string;
  /** A retired pair is shown, never chosen again. */
  disabled: boolean;
}

/**
 * The shoes row under an activity's map. It names the pair the activity
 * counts on — given by default or chosen — and opens the athlete's pairs as a
 * radio list, with "None of my pairs" last. An athlete with no pair is sent to
 * Settings to add one.
 */
export function ActivityGearPicker({ provider, activityId }: ActivityGearPickerProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const router = useRouter();
  const unit = useDistanceUnit();
  const ledger = useGearLedger();
  const gear = useActivityGear(provider, activityId, true);
  const [open, setOpen] = useState(false);
  const attribution = gear.attribution;

  if (ledger.isPending || gear.isLoading) {
    return (
      <View className="items-start px-4 py-3" accessibilityLabel={t('common.loading')} testID="activity-gear">
        <ActivityIndicator color={colors.tokens.primary} />
      </View>
    );
  }
  if (ledger.isError || gear.isError || attribution === undefined) {
    return (
      <Text className="px-4 text-sm text-text-secondary" testID="activity-gear">
        {describeApiError(ledger.error ?? gear.error, { t, fallbackKey: 'gear.loadFailed' })}
      </Text>
    );
  }

  const pairs = ledger.data.gear;
  const choices = gearChoices(pairs, attribution.gear_id);
  if (choices.length === 0 && attribution.gear_id === null) {
    return (
      <EmptyState
        testID="activity-gear"
        action={{
          label: t('gear.activity.addPairs'),
          onPress: () => router.push(GEAR_ROUTE as never),
          testID: 'activity-gear-add',
        }}
      >
        {t('gear.activity.noPairs')}
      </EmptyState>
    );
  }

  const chosen = pairs.find((pair) => pair.id === attribution.gear_id);
  const declaredNone = attribution.gear_id === null && attribution.source === 'declared';
  let current = t('gear.activity.choose');
  if (chosen !== undefined) {
    current = chosen.retired_at === null ? chosen.name : t('gear.activity.retiredOption', { name: chosen.name });
  } else if (declaredNone) {
    current = t('gear.activity.none');
  }

  const options: GearOption[] = [
    ...choices.map((pair) => ({
      key: pair.id,
      gearId: pair.id,
      label: pair.retired_at === null ? pair.name : t('gear.activity.retiredOption', { name: pair.name }),
      subtitle: sportLabel(t, pair.sport),
      value: formatGearDistance(pair.distance_meters, unit, language),
      disabled: pair.retired_at !== null,
    })),
    { key: 'none', gearId: null, label: t('gear.activity.none'), disabled: false },
  ];
  const isSelected = (option: GearOption) =>
    option.gearId === null ? declaredNone : option.gearId === attribution.gear_id;
  const pick = (option: GearOption) => {
    setOpen(false);
    if (!isSelected(option)) gear.choose(option.gearId);
  };

  return (
    <View testID="activity-gear">
      <Row
        title={t('gear.activity.label')}
        hint={current}
        hintTestID="activity-gear-value"
        onPress={gear.isChoosing ? undefined : () => setOpen(true)}
        trailing={gear.isChoosing ? <ActivityIndicator size="small" color={colors.text.tertiary} /> : undefined}
        accessibilityState={{ disabled: gear.isChoosing }}
        last
        testID="activity-gear-select"
      />
      {attribution.source === 'default' && (
        <Text className="px-4 text-xs text-text-secondary" testID="activity-gear-default">
          {t('gear.activity.default')}
        </Text>
      )}
      {gear.chooseFailed && (
        <Text className="mt-1 px-4 text-sm text-error" accessibilityRole="alert" testID="activity-gear-failed">
          {describeApiError(gear.chooseError, { t, fallbackKey: 'gear.activity.saveFailed' })}
        </Text>
      )}
      {chosen?.warn_reached && (
        <View className="px-4">
          <GearWarning pair={chosen} />
        </View>
      )}

      <Sheet visible={open} onClose={() => setOpen(false)} flush testID="activity-gear-sheet">
        <Text className="mb-2 px-4 text-lg font-semibold text-text-primary">{t('gear.activity.label')}</Text>
        <ScrollView showsVerticalScrollIndicator={false}>
          {options.map((option, index) => {
            const selected = isSelected(option);
            return (
              <Row
                key={option.key}
                title={option.label}
                subtitle={option.subtitle}
                value={option.value}
                onPress={option.disabled ? undefined : () => pick(option)}
                showChevron={false}
                last={index === options.length - 1}
                accessibilityRole="radio"
                accessibilityState={{ selected, disabled: option.disabled }}
                accessibilityLabel={option.label}
                trailing={selected ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined}
                testID={`activity-gear-option-${option.key}`}
              />
            );
          })}
        </ScrollView>
      </Sheet>
    </View>
  );
}
