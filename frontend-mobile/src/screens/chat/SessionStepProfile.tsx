// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The day editor's session profile on the phone — one bar per step, as wide as its time and as tall as its target, every round of a set drawn
// ABOUTME: A set wears a bracket with its × N; a first-round bar opens its step, the later rounds are quieter and never a stop (carnet#1727)

import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import type { PlanStep } from '@pierre/shared-types';
import {
  formatStepDuration,
  PROFILE_EFFORT_LEVEL,
  type ProfileBar,
  profileBlocks,
  SESSION_EDITOR_KEYS,
} from '@pierre/ui-logic';

/** A bar's height in points at level 0 and level 1. */
const BAR_MIN = 12;
const BAR_MAX = 64;

interface SessionStepProfileProps {
  steps: PlanStep[];
  /** The step whose editor is open, outlined in the profile. */
  openIndex: number | null;
  onOpen: (index: number) => void;
}

/**
 * The session at a glance, over its step list. Each bar is named by its
 * step, its time and its target, so the picture never speaks by colour or
 * height alone; only a set's first round is a button, its repeats are drawn
 * for the shape and hidden from a screen reader.
 */
export function SessionStepProfile({ steps, openIndex, onOpen }: SessionStepProfileProps) {
  const { t } = useTranslation();
  const name = (index: number): string => {
    const step = steps[index] as PlanStep;
    return t(SESSION_EDITOR_KEYS.profileStep, {
      number: index + 1,
      label: step.label,
      duration: formatStepDuration(step.duration_seconds),
      target: step.target_zone,
    });
  };
  const bar = (item: ProfileBar, key: string) => (
    <Bar
      key={key}
      bar={item}
      open={!item.repeated && item.index === openIndex}
      label={name(item.index)}
      onPress={() => onOpen(item.index)}
    />
  );
  return (
    <View
      className="mb-3 h-[88px] flex-row items-end gap-0.5 border-b border-outline"
      accessibilityRole="list"
      accessibilityLabel={t(SESSION_EDITOR_KEYS.profile)}
      testID="session-editor-profile"
    >
      {profileBlocks(steps).map((block) =>
        block.kind === 'step' ? (
          bar(block.bar, `step-${block.bar.index}`)
        ) : (
          <View
            key={`set-${block.start}`}
            className="h-full justify-end"
            // A set is as wide as all its rounds: measured from the steps, not classed.
            style={{ flex: block.bars.reduce((sum, item) => sum + item.seconds, 0) }}
            testID={`session-editor-profile-set-${block.start}`}
          >
            <View className="mb-1 border-t-2 border-on-surface pt-0.5">
              <Text
                className="font-mono text-xs text-text-primary"
                accessibilityElementsHidden
                importantForAccessibility="no"
              >
                × {block.repeat}
              </Text>
            </View>
            <View className="h-16 flex-row items-end gap-0.5">
              {block.bars.map((item, round) => bar(item, ['set', block.start, round].join('-')))}
            </View>
          </View>
        ),
      )}
    </View>
  );
}

interface BarProps {
  bar: ProfileBar;
  /** The bar of the step whose editor is open. */
  open: boolean;
  label: string;
  onPress: () => void;
}

/**
 * One bar. Its width is its share of the session's time and its height its
 * target's level, so both are measured, not classed; the column it stands in
 * is the touch target, the full height of the profile.
 */
function Bar({ bar, open, label, onPress }: BarProps) {
  const height = Math.round(BAR_MIN + bar.level * (BAR_MAX - BAR_MIN));
  const fill = `rounded-t-sm ${bar.level >= PROFILE_EFFORT_LEVEL ? 'bg-primary' : 'bg-outline-variant'}${
    open ? ' border-2 border-on-surface' : ''
  }`;
  if (bar.repeated) {
    return (
      <View
        className="h-full justify-end opacity-60"
        style={{ flex: bar.seconds }}
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
      >
        <View className={fill} style={{ height }} />
      </View>
    );
  }
  return (
    <Pressable
      className="h-full justify-end"
      style={{ flex: bar.seconds }}
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ selected: open }}
      testID={`session-editor-profile-step-${bar.index}`}
    >
      <View className={fill} style={{ height }} />
    </Pressable>
  );
}
