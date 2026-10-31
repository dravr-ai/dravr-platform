// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The "Mes athlètes / Mon entraînement" switch a coach who also trains finds at the top of Home (D2, carnet#1253)
// ABOUTME: Two pressed-state buttons in one pill; the first is the coach's Home, the second the athlete Home every athlete has

import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';

export type HomeScope = 'athletes' | 'me';

interface CoachHomeSwitchProps {
  scope: HomeScope;
  onChange: (scope: HomeScope) => void;
}

export function CoachHomeSwitch({ scope, onChange }: CoachHomeSwitchProps) {
  const { t } = useTranslation();
  const items: { id: HomeScope; label: string }[] = [
    { id: 'athletes', label: t('coachHome.switch.athletes') },
    { id: 'me', label: t('coachHome.switch.me') },
  ];
  return (
    <View
      accessibilityRole="tablist"
      accessibilityLabel={t('coachHome.switch.aria')}
      testID="coach-home-switch"
      className="flex-row rounded-xl bg-surface-container-high p-0.5"
    >
      {items.map((item) => {
        const selected = scope === item.id;
        return (
          <Pressable
            key={item.id}
            accessibilityRole="tab"
            accessibilityState={{ selected }}
            onPress={() => !selected && onChange(item.id)}
            testID={`coach-home-switch-${item.id}`}
            className={`h-9 flex-1 items-center justify-center rounded-lg ${
              selected ? 'border border-border bg-surface-container-lowest' : 'border border-transparent'
            }`}
          >
            <Text className={`text-sm ${selected ? 'font-semibold text-text-primary' : 'font-medium text-text-secondary'}`}>
              {item.label}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}
