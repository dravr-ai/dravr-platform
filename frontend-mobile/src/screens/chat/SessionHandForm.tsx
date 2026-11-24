// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The day editor's "Write" mode on the phone — the session's duration and intensity, or its steps: a profile, sets under one × N, one step open at a time
// ABOUTME: Works without the family catalogue; holds only what the coach wrote, through the shared hand-session operations (carnet#1714, #1716, #1727)

import React, { useState } from 'react';
import { type AccessibilityActionEvent, Pressable, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import type { SessionSport } from '@pierre/shared-types';
import {
  addSeries,
  addStep,
  addStepToSeries,
  duplicateStep,
  type HandSession,
  handStatesDuration,
  moveStep,
  removeStep,
  SERIES_MAX_REPEAT,
  SERIES_MIN_REPEAT,
  seriesDefaults,
  SESSION_EDITOR_KEYS,
  setHand,
  setSeriesRepeat,
  setStepMeasureOf,
  type StepBlock,
  stepBlocks,
  stepMoveTarget,
  updateStep,
} from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import { Input, SquareIconButton } from '../../components/ui';
import { StepCard, StepRow } from './SessionStepCard';
import { SessionStepProfile } from './SessionStepProfile';

interface SessionHandFormProps {
  session: HandSession;
  /** The family sport the session's sport reads as (`familySportFor`): a new set's zones, a percent target's anchor. */
  familySport: SessionSport | undefined;
  onChange: (session: HandSession) => void;
}

/** Where a screen reader's focus lands after a reshape: a step's row, or the moved step's move button. */
type Refocus = { kind: 'row'; index: number } | { kind: 'move'; index: number; delta: -1 | 1 };

/** Digits only: a duration in minutes, never a sign or a decimal. */
function wholeNumber(text: string): number | undefined {
  const digits = text.replace(/[^0-9]/g, '');
  return digits === '' ? undefined : Number(digits);
}

/**
 * The session the coach writes. Without steps, its duration and intensity;
 * with them, the profile over the step list — a lone step a collapsed row, a
 * set a ruled group under its × N stepper — and one step open at a time. A
 * step just added opens with the caret in its name. A session the coach
 * writes names no family, so the form closes on the line saying it is not
 * checked against one.
 */
export function SessionHandForm({ session, familySport, onChange }: SessionHandFormProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState<number | null>(null);
  // The step whose name field takes the caret as it opens: one just added.
  const [focus, setFocus] = useState<number | null>(null);
  // Bumped on every reshape, so the rows and the open card are drawn afresh
  // for the steps now at their indices rather than keeping the text of the
  // ones that were there.
  const [shape, setShape] = useState(0);
  // Where a screen reader's focus goes once a move or a remove reshapes the
  // list: the moved step's move button, or the step that took a removed one's place.
  const [refocus, setRefocus] = useState<Refocus | null>(null);
  const blocks = stepBlocks(session.steps);

  const reshape = (next: HandSession, opened: number | null, focused = false, then: Refocus | null = null) => {
    setShape((current) => current + 1);
    setOpen(opened);
    setFocus(focused ? opened : null);
    setRefocus(then);
    onChange(next);
  };
  const openStep = (index: number) => {
    setOpen(index);
    setFocus(null);
    setRefocus(null);
  };
  const remove = (index: number) => {
    const left = session.steps.length - 1;
    const next = index < left ? index : index - 1;
    reshape(removeStep(session, index), null, false, next >= 0 ? { kind: 'row', index: next } : null);
  };

  const repeatSet = (block: Extract<StepBlock, { kind: 'series' }>, repeat: number) =>
    reshape(setSeriesRepeat(session, block.start, repeat), open);

  const renderStep = (index: number) => {
    const step = session.steps[index];
    if (!step) return null;
    if (open !== index) {
      return (
        <StepRow
          key={`row-${index}-${shape}`}
          step={step}
          index={index}
          focusOnMount={refocus?.kind === 'row' && refocus.index === index}
          onOpen={() => openStep(index)}
        />
      );
    }
    const up = stepMoveTarget(session.steps, index, -1);
    const down = stepMoveTarget(session.steps, index, 1);
    return (
      <StepCard
        key={`open-${index}-${shape}`}
        step={step}
        index={index}
        familySport={familySport}
        canMoveUp={up !== null}
        canMoveDown={down !== null}
        autoFocus={focus === index}
        focusMove={refocus?.kind === 'move' && refocus.index === index ? refocus.delta : undefined}
        onChange={(patch) => onChange(updateStep(session, index, patch))}
        onMeasure={(measure) => onChange(setStepMeasureOf(session, index, measure))}
        onMove={(delta) => {
          const to = delta < 0 ? up : down;
          if (to !== null) reshape(moveStep(session, index, delta), to, false, { kind: 'move', index: to, delta });
        }}
        onDuplicate={() => reshape(duplicateStep(session, index), index + 1)}
        onRemove={() => remove(index)}
      />
    );
  };

  return (
    <View className="px-4" testID="session-editor-hand">
      {/* With steps the server writes the duration they total, and the
          header names the span their targets cover: the coach states both
          only for a session without steps. */}
      {handStatesDuration(session) ? (
        <View className="flex-row gap-4">
          <View className="w-32">
            <Input
              label={t(SESSION_EDITOR_KEYS.duration)}
              value={session.duration_min === undefined ? '' : String(session.duration_min)}
              onChangeText={(text) => onChange(setHand(session, { duration_min: wholeNumber(text) }))}
              keyboardType="number-pad"
              accessibilityLabel={t(SESSION_EDITOR_KEYS.duration)}
              testID="session-editor-duration"
            />
          </View>
          <View className="flex-1">
            <Input
              label={t(SESSION_EDITOR_KEYS.intensity)}
              value={session.intensity}
              onChangeText={(intensity) => onChange(setHand(session, { intensity }))}
              accessibilityLabel={t(SESSION_EDITOR_KEYS.intensity)}
              testID="session-editor-intensity"
            />
          </View>
        </View>
      ) : null}

      <Text className="mb-2 mt-1 text-sm font-medium text-text-secondary" accessibilityRole="header">
        {t(SESSION_EDITOR_KEYS.stepsHeading)}
      </Text>
      {session.steps.length === 0 ? (
        <Text className="pb-2 text-sm text-text-secondary" testID="session-editor-no-steps">
          {t(SESSION_EDITOR_KEYS.noSteps)}
        </Text>
      ) : (
        <>
          <SessionStepProfile steps={session.steps} openIndex={open} onOpen={openStep} />
          {blocks.map((block) => {
            if (block.kind === 'step') return renderStep(block.index);
            const indices = Array.from({ length: block.end - block.start }, (_, at) => block.start + at);
            return (
              <View
                key={`set-${block.start}`}
                className="my-1.5 border-l-2 border-on-surface pl-3"
                testID={`session-editor-set-${block.start}`}
              >
                <View className="min-h-11 flex-row items-center justify-between">
                  <Text className="text-sm font-semibold text-text-secondary">{t(SESSION_EDITOR_KEYS.series)}</Text>
                  <View className="flex-row items-center gap-1">
                    <SquareIconButton
                      icon="minus"
                      label={t(SESSION_EDITOR_KEYS.seriesFewer)}
                      onPress={() => repeatSet(block, block.repeat - 1)}
                      testID={`session-editor-set-${block.start}-fewer`}
                    />
                    <Text
                      className="w-11 text-center font-mono text-md text-text-primary"
                      accessible
                      accessibilityRole="adjustable"
                      accessibilityLabel={t(SESSION_EDITOR_KEYS.seriesRepeat)}
                      accessibilityValue={{
                        min: SERIES_MIN_REPEAT,
                        max: SERIES_MAX_REPEAT,
                        now: block.repeat,
                        text: `× ${block.repeat}`,
                      }}
                      accessibilityActions={[{ name: 'increment' }, { name: 'decrement' }]}
                      onAccessibilityAction={(event: AccessibilityActionEvent) =>
                        repeatSet(block, block.repeat + (event.nativeEvent.actionName === 'increment' ? 1 : -1))
                      }
                      testID={`session-editor-set-${block.start}-repeat`}
                    >
                      {`× ${block.repeat}`}
                    </Text>
                    <SquareIconButton
                      icon="plus"
                      label={t(SESSION_EDITOR_KEYS.seriesMore)}
                      disabled={block.repeat >= SERIES_MAX_REPEAT}
                      onPress={() => repeatSet(block, block.repeat + 1)}
                      testID={`session-editor-set-${block.start}-more`}
                    />
                  </View>
                </View>
                {indices.map(renderStep)}
                <AddButton
                  icon="plus"
                  label={t(SESSION_EDITOR_KEYS.addStepToSeries)}
                  onPress={() => reshape(addStepToSeries(session, block.start), block.end, true)}
                  testID={`session-editor-set-${block.start}-add-step`}
                />
              </View>
            );
          })}
        </>
      )}
      <View className="flex-row flex-wrap gap-x-5">
        <AddButton
          icon="plus"
          label={t(SESSION_EDITOR_KEYS.addStep)}
          onPress={() => reshape(addStep(session), session.steps.length, true)}
          testID="session-editor-add-step"
        />
        <AddButton
          icon="repeat"
          label={t(SESSION_EDITOR_KEYS.addSeries)}
          onPress={() =>
            reshape(
              addSeries(
                session,
                seriesDefaults(familySport, {
                  work: t(SESSION_EDITOR_KEYS.seriesWork),
                  recovery: t(SESSION_EDITOR_KEYS.seriesRecovery),
                }),
              ),
              session.steps.length,
              true,
            )
          }
          testID="session-editor-add-series"
        />
      </View>

      <Text className="mt-3 text-sm text-text-secondary" testID="session-editor-unchecked">
        {t(SESSION_EDITOR_KEYS.unchecked)}
      </Text>
    </View>
  );
}

interface AddButtonProps {
  icon: 'plus' | 'repeat';
  label: string;
  onPress: () => void;
  testID: string;
}

/** A quiet 44pt add in the primary ink: a step, a set, a step in a set. */
function AddButton({ icon, label, onPress, testID }: AddButtonProps) {
  const colors = useThemeColors();
  return (
    <Pressable
      className="min-h-11 flex-row items-center self-start"
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      testID={testID}
    >
      <Feather name={icon} size={16} color={colors.tokens.primary} />
      <Text className="ml-1.5 text-sm font-semibold text-primary">{label}</Text>
    </Pressable>
  );
}
