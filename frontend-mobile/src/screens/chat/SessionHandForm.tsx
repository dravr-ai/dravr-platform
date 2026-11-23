// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The day editor's "This session" mode on the phone — one session edited in place, or a new one written: rest, title, sport, duration, intensity, steps
// ABOUTME: Works without the family catalogue; holds only what the coach wrote, through the shared hand-session operations (carnet#1714, #1716)

import React, { useState } from 'react';
import { Pressable, StyleSheet, Switch, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import type { PlanStep } from '@pierre/shared-types';
import {
  addStep,
  formatStepDuration,
  type HandSession,
  handStatesDuration,
  moveStep,
  parseStepDuration,
  removeStep,
  SESSION_EDITOR_KEYS,
  setHand,
  sportLabel,
  updateStep,
} from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import { Input, RadioGroup, switchColors } from '../../components/ui';

interface SessionHandFormProps {
  session: HandSession;
  /** The sports offered, fixed for the sheet's life so a pick never reorders the row under the thumb. */
  sports: string[];
  /**
   * Whether the Rest switch shows: on an entry being edited, not on a session
   * being added to a date — a date's rest is its own entry (carnet#1716).
   */
  restOffered?: boolean;
  onChange: (session: HandSession) => void;
}

/** Digits only: a duration in minutes or a repeat count, never a sign or a decimal. */
function wholeNumber(text: string): number | undefined {
  const digits = text.replace(/[^0-9]/g, '');
  return digits === '' ? undefined : Number(digits);
}

/**
 * The entry's own session. On a rest day only the Rest switch shows; switched
 * off, the session's fields come back as they were. A new session has no
 * Rest switch. A session the coach edits names no family, so the form closes
 * on the line saying it is not checked against one.
 */
export function SessionHandForm({
  session,
  sports,
  restOffered = true,
  onChange,
}: SessionHandFormProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  // The step duration being typed, as typed: the field keeps the coach's
  // text while it has focus and shows the step's m:ss once it loses it.
  const [typing, setTyping] = useState<{ index: number; text: string } | null>(
    null,
  );

  // A step added, moved or removed shifts the rows under the field being
  // typed into, so its text is let go rather than shown on another step.
  const reshape = (next: HandSession) => {
    setTyping(null);
    onChange(next);
  };

  return (
    <View testID="session-editor-hand">
      {restOffered ? (
        <View className="mb-2 min-h-11 flex-row items-center justify-between gap-4 px-4">
          <Text className="flex-1 text-base text-text-primary">
            {t(SESSION_EDITOR_KEYS.restDay)}
          </Text>
          <Switch
            value={session.rest}
            onValueChange={(rest) => onChange(setHand(session, { rest }))}
            accessibilityLabel={t(SESSION_EDITOR_KEYS.restDay)}
            {...switchColors(colors.tokens, session.rest)}
            testID="session-editor-rest"
          />
        </View>
      ) : null}

      {session.rest ? null : (
        <View className="px-4">
          <Input
            label={t(SESSION_EDITOR_KEYS.workout)}
            value={session.workout}
            onChangeText={(workout) => onChange(setHand(session, { workout }))}
            accessibilityLabel={t(SESSION_EDITOR_KEYS.workout)}
            returnKeyType="done"
            testID="session-editor-workout"
          />
          <View className="mb-4">
            <RadioGroup
              layout="segmented"
              label={t(SESSION_EDITOR_KEYS.sport)}
              options={sports.map((sport) => ({
                key: sport,
                label: sportLabel(t, sport),
              }))}
              value={session.sport}
              onChange={(sport) => onChange(setHand(session, { sport }))}
              testID="session-editor-hand-sport"
            />
          </View>
          <View className="flex-row gap-4">
            {/* With steps the server writes the duration they total, so the
                coach states it only for a session without steps. */}
            {handStatesDuration(session) ? (
              <View className="w-32">
                <Input
                  label={t(SESSION_EDITOR_KEYS.duration)}
                  value={
                    session.duration_min === undefined
                      ? ''
                      : String(session.duration_min)
                  }
                  onChangeText={(text) =>
                    onChange(
                      setHand(session, { duration_min: wholeNumber(text) }),
                    )
                  }
                  keyboardType="number-pad"
                  accessibilityLabel={t(SESSION_EDITOR_KEYS.duration)}
                  testID="session-editor-duration"
                />
              </View>
            ) : null}
            <View className="flex-1">
              <Input
                label={t(SESSION_EDITOR_KEYS.intensity)}
                value={session.intensity}
                onChangeText={(intensity) =>
                  onChange(setHand(session, { intensity }))
                }
                accessibilityLabel={t(SESSION_EDITOR_KEYS.intensity)}
                testID="session-editor-intensity"
              />
            </View>
          </View>

          <Text
            className="mb-1 mt-2 text-sm font-semibold text-text-primary"
            accessibilityRole="header"
          >
            {t(SESSION_EDITOR_KEYS.stepsHeading)}
          </Text>
          {session.steps.length === 0 ? (
            <Text
              className="py-2 text-sm text-text-secondary"
              testID="session-editor-no-steps"
            >
              {t(SESSION_EDITOR_KEYS.noSteps)}
            </Text>
          ) : (
            session.steps.map((step, index) => (
              <StepRow
                // Steps carry no identity of their own; the typed text is
                // let go on every reshape, so the index is a safe key.
                key={index}
                step={step}
                index={index}
                count={session.steps.length}
                durationText={
                  typing?.index === index
                    ? typing.text
                    : formatStepDuration(step.duration_seconds)
                }
                onFocusDuration={() =>
                  setTyping({
                    index,
                    text: formatStepDuration(step.duration_seconds),
                  })
                }
                onChangeDuration={(text) => {
                  setTyping({ index, text });
                  const seconds = parseStepDuration(text);
                  if (seconds !== undefined)
                    onChange(
                      updateStep(session, index, { duration_seconds: seconds }),
                    );
                }}
                // Only this step's text is let go: a platform that reports the
                // next field's focus before this one's blur keeps the new text.
                onBlurDuration={() =>
                  setTyping((current) =>
                    current?.index === index ? null : current,
                  )
                }
                onChange={(patch) =>
                  onChange(updateStep(session, index, patch))
                }
                onMove={(delta) => reshape(moveStep(session, index, delta))}
                onRemove={() => reshape(removeStep(session, index))}
              />
            ))
          )}
          <Pressable
            className="min-h-11 flex-row items-center self-start"
            accessibilityRole="button"
            accessibilityLabel={t(SESSION_EDITOR_KEYS.addStep)}
            onPress={() => reshape(addStep(session))}
            testID="session-editor-add-step"
          >
            <Feather name="plus" size={16} color={colors.tokens.primary} />
            <Text className="ml-1.5 text-sm font-medium text-primary">
              {t(SESSION_EDITOR_KEYS.addStep)}
            </Text>
          </Pressable>

          <Text
            className="mt-3 text-sm text-text-secondary"
            testID="session-editor-unchecked"
          >
            {t(SESSION_EDITOR_KEYS.unchecked)}
          </Text>
        </View>
      )}
    </View>
  );
}

interface StepRowProps {
  step: PlanStep;
  index: number;
  count: number;
  /** What the duration field shows: the text being typed, else the step's m:ss. */
  durationText: string;
  onFocusDuration: () => void;
  onChangeDuration: (text: string) => void;
  onBlurDuration: () => void;
  onChange: (patch: Partial<PlanStep>) => void;
  onMove: (delta: -1 | 1) => void;
  onRemove: () => void;
}

/**
 * One numbered step: its number and its three controls on one line, then the
 * label and target, then the duration in m:ss and the repeat count. Each
 * control is a 44pt target named with the step's number, so a screen reader
 * tells "move step 2 up" from "move step 3 up".
 */
function StepRow({
  step,
  index,
  count,
  durationText,
  onFocusDuration,
  onChangeDuration,
  onBlurDuration,
  onChange,
  onMove,
  onRemove,
}: StepRowProps) {
  const { t } = useTranslation();
  const number = index + 1;
  return (
    <View
      className="border-t border-border-faint pt-1"
      style={{ borderTopWidth: StyleSheet.hairlineWidth }}
      testID={`session-editor-step-${index}`}
    >
      <View className="flex-row items-center">
        <Text className="flex-1 font-mono text-sm font-semibold text-text-primary">
          {number}
        </Text>
        <StepControl
          icon="arrow-up"
          label={t(SESSION_EDITOR_KEYS.moveStepUp, { number })}
          disabled={index === 0}
          onPress={() => onMove(-1)}
          testID={`session-editor-step-${index}-up`}
        />
        <StepControl
          icon="arrow-down"
          label={t(SESSION_EDITOR_KEYS.moveStepDown, { number })}
          disabled={index === count - 1}
          onPress={() => onMove(1)}
          testID={`session-editor-step-${index}-down`}
        />
        <StepControl
          icon="trash-2"
          label={t(SESSION_EDITOR_KEYS.removeStep, { number })}
          onPress={onRemove}
          testID={`session-editor-step-${index}-remove`}
        />
      </View>
      <View className="flex-row gap-4">
        <View style={{ flex: 3 }}>
          <Input
            label={t(SESSION_EDITOR_KEYS.stepLabel)}
            value={step.label}
            onChangeText={(label) => onChange({ label })}
            accessibilityLabel={t(SESSION_EDITOR_KEYS.stepLabel)}
            testID={`session-editor-step-${index}-label`}
          />
        </View>
        <View style={{ flex: 2 }}>
          <Input
            label={t(SESSION_EDITOR_KEYS.stepTarget)}
            value={step.target_zone}
            onChangeText={(target_zone) => onChange({ target_zone })}
            accessibilityLabel={t(SESSION_EDITOR_KEYS.stepTarget)}
            autoCapitalize="none"
            testID={`session-editor-step-${index}-target`}
          />
        </View>
      </View>
      <View className="flex-row gap-4">
        <View className="flex-1">
          <Input
            label={t(SESSION_EDITOR_KEYS.stepDuration)}
            value={durationText}
            onFocus={onFocusDuration}
            onChangeText={onChangeDuration}
            onBlur={onBlurDuration}
            keyboardType="numbers-and-punctuation"
            accessibilityLabel={t(SESSION_EDITOR_KEYS.stepDuration)}
            testID={`session-editor-step-${index}-duration`}
          />
        </View>
        <View className="flex-1">
          <Input
            label={t(SESSION_EDITOR_KEYS.stepRepeat)}
            value={step.repeat === undefined ? '' : String(step.repeat)}
            onChangeText={(text) => onChange({ repeat: wholeNumber(text) })}
            keyboardType="number-pad"
            accessibilityLabel={t(SESSION_EDITOR_KEYS.stepRepeat)}
            testID={`session-editor-step-${index}-repeat`}
          />
        </View>
      </View>
    </View>
  );
}

interface StepControlProps {
  icon: 'arrow-up' | 'arrow-down' | 'trash-2';
  label: string;
  disabled?: boolean;
  onPress: () => void;
  testID: string;
}

/** A 44pt icon control on a step, in the secondary ink; dimmed and inert at the list's ends. */
function StepControl({
  icon,
  label,
  disabled = false,
  onPress,
  testID,
}: StepControlProps) {
  const colors = useThemeColors();
  return (
    <Pressable
      className={`h-11 w-11 items-center justify-center ${disabled ? 'opacity-40' : ''}`}
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      testID={testID}
    >
      <Feather name={icon} size={18} color={colors.text.secondary} />
    </Pressable>
  );
}
