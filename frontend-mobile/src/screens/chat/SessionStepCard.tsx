// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One step of a hand-written session on the phone — a collapsed row of its name and tags, or the open card that edits it
// ABOUTME: The card holds the name, time or distance, the m:ss field with −/+ 15 s, and the target chips that write the server's grammar (carnet#1727)

import React, { forwardRef, useEffect, useRef, useState } from 'react';
import { AccessibilityInfo, Pressable, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import type { PlanStep, SessionSport } from '@pierre/shared-types';
import {
  defaultAnchor,
  formatStepDuration,
  MAX_TARGET_ZONE,
  parseStepDuration,
  parseTarget,
  percentTarget,
  rpeTarget,
  SESSION_EDITOR_KEYS,
  type StepMeasure,
  stepMeasure,
  TARGET_ANCHOR_LABEL_KEY,
  TARGET_ANCHORS,
  type TargetAnchor,
  type Translate,
  zoneTarget,
} from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import { Chip, Input, RadioGroup, StepperField } from '../../components/ui';

/** What the −/+ buttons beside a step's time move it by. */
const NUDGE_SECONDS = 15;

/** The zones the target chips offer, `Z1` to `Z7`. */
const ZONES = Array.from({ length: MAX_TARGET_ZONE }, (_, at) => at + 1);

/** Digits only: a distance, a percent or an RPE, never a sign or a decimal. */
function digits(text: string): string {
  return text.replace(/[^0-9]/g, '');
}

/** A distance in metres as a tag writes it, in the coach's notation (`1 200 m` in French). */
function metres(t: Translate, distance: number, language: string): string {
  const value = new Intl.NumberFormat(language, { maximumFractionDigits: 0 }).format(distance);
  return t(SESSION_EDITOR_KEYS.distanceTag, { value });
}

/** The tags a collapsed row prints after the step's name: its distance when it has one, its time, its target. */
function stepTags(t: Translate, step: PlanStep, language: string): string[] {
  const tags: string[] = [];
  if (step.distance_meters !== undefined && step.distance_meters > 0) tags.push(metres(t, step.distance_meters, language));
  tags.push(formatStepDuration(step.duration_seconds));
  if (step.target_zone.trim().length > 0) tags.push(step.target_zone.trim());
  return tags;
}

interface StepRowProps {
  step: PlanStep;
  index: number;
  /** A screen reader's focus lands here as the row appears: the step that took a removed one's place. */
  focusOnMount?: boolean;
  onOpen: () => void;
}

/**
 * A step at rest: its number, its name and its tags on one 48pt row, the
 * whole row one button that opens the step's editor.
 */
export function StepRow({ step, index, focusOnMount = false, onOpen }: StepRowProps) {
  const { t, language } = useTranslation();
  const row = useRef<View>(null);
  useEffect(() => {
    if (focusOnMount && row.current) AccessibilityInfo.sendAccessibilityEvent(row.current, 'focus');
  }, [focusOnMount]);
  const number = index + 1;
  const tags = stepTags(t, step, language);
  const named = step.label.trim().length > 0;
  return (
    <Pressable
      ref={row}
      className="min-h-12 flex-row items-center gap-2.5 border-b border-border-faint py-1.5 active:bg-surface-container-low"
      onPress={onOpen}
      accessibilityRole="button"
      accessibilityLabel={t(SESSION_EDITOR_KEYS.editStep, { number, label: step.label.trim() })}
      accessibilityHint={tags.join(', ')}
      testID={`session-editor-step-${index}`}
    >
      <Text className="w-6 text-right font-mono text-xs text-text-secondary">{number}</Text>
      <Text
        className={`flex-1 text-md font-medium ${named ? 'text-text-primary' : 'text-text-tertiary'}`}
        numberOfLines={1}
      >
        {named ? step.label : t(SESSION_EDITOR_KEYS.stepLabel)}
      </Text>
      <View className="flex-row gap-1">
        {tags.map((tag, at) => (
          <View key={`${at}-${tag}`} className="h-6 justify-center rounded-md bg-surface-container-low px-1.5">
            <Text className="font-mono text-xs text-text-primary">{tag}</Text>
          </View>
        ))}
      </View>
    </Pressable>
  );
}

interface StepCardProps {
  step: PlanStep;
  index: number;
  /** The family sport the session reads as: a percent target starts on its anchor. */
  familySport: SessionSport | undefined;
  /** Whether a move up or down would change anything; a move that would not is dimmed and inert. */
  canMoveUp: boolean;
  canMoveDown: boolean;
  /** A step just added: its name field takes the caret. */
  autoFocus: boolean;
  /**
   * The step was just moved this way: a screen reader's focus stays on the
   * move it pressed, or on the other one once this way is spent.
   */
  focusMove?: -1 | 1;
  onChange: (patch: Partial<PlanStep>) => void;
  onMeasure: (measure: StepMeasure) => void;
  onMove: (delta: -1 | 1) => void;
  onDuplicate: () => void;
  onRemove: () => void;
}

/**
 * The open step, in a card with a primary edge: its number and its four
 * actions, its name, how it is measured, its time (or distance and an
 * estimated time), and its target. Each action is a 44pt target named with
 * the step's number, so a screen reader tells "move step 2 up" from "move
 * step 3 up".
 */
export function StepCard({
  step,
  index,
  familySport,
  canMoveUp,
  canMoveDown,
  autoFocus,
  focusMove,
  onChange,
  onMeasure,
  onMove,
  onDuplicate,
  onRemove,
}: StepCardProps) {
  const { t } = useTranslation();
  const up = useRef<View>(null);
  const down = useRef<View>(null);
  useEffect(() => {
    if (focusMove === undefined) return;
    const pressed = focusMove < 0 ? canMoveUp : canMoveDown;
    const target = (focusMove < 0) === pressed ? up.current : down.current;
    if (target) AccessibilityInfo.sendAccessibilityEvent(target, 'focus');
    // The card is drawn afresh on every move, so this runs as it appears.
  }, [focusMove, canMoveUp, canMoveDown]);
  const number = index + 1;
  const measure = stepMeasure(step);
  const id = `session-editor-step-${index}`;
  return (
    <View className="mb-1.5 rounded-xl border border-primary bg-surface-container-lowest px-3 pb-1" testID={`${id}-open`}>
      <View className="flex-row items-center">
        <Text className="flex-1 font-mono text-sm font-semibold text-text-primary">{number}</Text>
        <StepControl
          ref={up}
          icon="arrow-up"
          label={t(SESSION_EDITOR_KEYS.moveStepUp, { number })}
          disabled={!canMoveUp}
          onPress={() => onMove(-1)}
          testID={`${id}-up`}
        />
        <StepControl
          ref={down}
          icon="arrow-down"
          label={t(SESSION_EDITOR_KEYS.moveStepDown, { number })}
          disabled={!canMoveDown}
          onPress={() => onMove(1)}
          testID={`${id}-down`}
        />
        <StepControl
          icon="copy"
          label={t(SESSION_EDITOR_KEYS.duplicateStep, { number })}
          onPress={onDuplicate}
          testID={`${id}-duplicate`}
        />
        <StepControl
          icon="trash-2"
          label={t(SESSION_EDITOR_KEYS.removeStep, { number })}
          onPress={onRemove}
          testID={`${id}-remove`}
        />
      </View>
      <Input
        label={t(SESSION_EDITOR_KEYS.stepLabel)}
        value={step.label}
        onChangeText={(label) => onChange({ label })}
        accessibilityLabel={t(SESSION_EDITOR_KEYS.stepLabel)}
        autoFocus={autoFocus}
        returnKeyType="done"
        testID={`${id}-label`}
      />
      <View className="mb-4">
        <RadioGroup
          layout="segmented"
          accessibilityLabel={t(SESSION_EDITOR_KEYS.measure)}
          options={[
            { key: 'time', label: t(SESSION_EDITOR_KEYS.measureTime) },
            { key: 'distance', label: t(SESSION_EDITOR_KEYS.measureDistance) },
          ]}
          value={measure}
          onChange={(key) => onMeasure(key === 'distance' ? 'distance' : 'time')}
          testID={`${id}-measure`}
        />
      </View>
      {measure === 'distance' ? (
        <Input
          label={t(SESSION_EDITOR_KEYS.stepDistance)}
          value={step.distance_meters ? String(step.distance_meters) : ''}
          onChangeText={(text) => {
            const typed = digits(text);
            // An emptied field keeps the step measured by distance: the save waits for one.
            onChange({ distance_meters: typed === '' ? 0 : Number(typed) });
          }}
          keyboardType="number-pad"
          accessibilityLabel={t(SESSION_EDITOR_KEYS.stepDistance)}
          testID={`${id}-distance`}
        />
      ) : null}
      <DurationField
        label={t(measure === 'distance' ? SESSION_EDITOR_KEYS.stepEstimate : SESSION_EDITOR_KEYS.stepDuration)}
        seconds={step.duration_seconds}
        nudge={measure === 'time'}
        onChange={(duration_seconds) => onChange({ duration_seconds })}
        testID={`${id}-duration`}
      />
      <TargetPicker
        index={index}
        target={step.target_zone}
        familySport={familySport}
        onChange={(target_zone) => onChange({ target_zone })}
      />
    </View>
  );
}

interface DurationFieldProps {
  label: string;
  seconds: number;
  /** Whether −/+ 15 s buttons flank the field: on a step measured by time. */
  nudge: boolean;
  onChange: (seconds: number) => void;
  testID: string;
}

/**
 * A step's time in m:ss. The field keeps the coach's text while it has
 * focus and shows the step's m:ss once it loses it; it stays a text keyboard,
 * since a number pad has no colon. The −/+ buttons move the time by 15 s for
 * a thumb that would rather not type.
 */
function DurationField({ label, seconds, nudge, onChange, testID }: DurationFieldProps) {
  const { t } = useTranslation();
  const [typing, setTyping] = useState<string | null>(null);
  const move = (delta: number) => {
    setTyping(null);
    onChange(Math.max(0, seconds + delta));
  };
  return (
    <View className="mb-4">
      <Text className="mb-2 text-sm font-medium text-text-secondary">{label}</Text>
      <StepperField
        value={typing ?? formatStepDuration(seconds)}
        onFocus={() => setTyping(formatStepDuration(seconds))}
        onChangeText={(text) => {
          setTyping(text);
          const parsed = parseStepDuration(text);
          if (parsed !== undefined) onChange(parsed);
        }}
        onBlur={() => setTyping(null)}
        keyboardType="numbers-and-punctuation"
        accessibilityLabel={label}
        testID={testID}
        decrement={
          nudge
            ? {
                label: t(SESSION_EDITOR_KEYS.durationLess),
                disabled: seconds <= 0,
                onPress: () => move(-NUDGE_SECONDS),
                testID: `${testID}-less`,
              }
            : undefined
        }
        increment={
          nudge
            ? {
                label: t(SESSION_EDITOR_KEYS.durationMore),
                onPress: () => move(NUDGE_SECONDS),
                testID: `${testID}-more`,
              }
            : undefined
        }
      />
    </View>
  );
}

/** Which of the target chips is picked: none yet, a zone, a percent band, an RPE band, or text as written. */
type TargetPanel = 'none' | 'zone' | 'percent' | 'rpe' | 'typed';

interface TargetPickerProps {
  index: number;
  /** The step's target as written. */
  target: string;
  familySport: SessionSport | undefined;
  onChange: (target: string) => void;
}

/**
 * The target chips — Z1 to Z7, "%", "RPE", "Other" — picked from what the
 * target reads as. A zone writes itself; "%" opens From / To and the
 * threshold it is of; "RPE" opens its value(s) on the 1–10 scale; "Other"
 * opens the target as written, where an agent's "sweet spot" lands. Until a
 * picked band has a value the target is empty, and the save says so.
 */
function TargetPicker({ index, target, familySport, onChange }: TargetPickerProps) {
  const { t } = useTranslation();
  const parsed = parseTarget(target);
  const [panel, setPanel] = useState<TargetPanel>(parsed.kind);
  const [from, setFrom] = useState(parsed.kind === 'percent' ? String(parsed.low) : '');
  const [to, setTo] = useState(parsed.kind === 'percent' && parsed.high !== parsed.low ? String(parsed.high) : '');
  const [anchor, setAnchor] = useState<TargetAnchor | null>(
    parsed.kind === 'percent' ? parsed.anchor : defaultAnchor(familySport),
  );
  const [rpeLow, setRpeLow] = useState(parsed.kind === 'rpe' ? String(parsed.low) : '');
  const [rpeHigh, setRpeHigh] = useState(parsed.kind === 'rpe' && parsed.high !== parsed.low ? String(parsed.high) : '');
  const id = ['session-editor-step', index, 'target'].join('-');

  const writePercent = (low: string, high: string, of: TargetAnchor | null) =>
    onChange(low === '' ? '' : percentTarget(Number(low), high === '' ? Number(low) : Number(high), of));
  const writeRpe = (low: string, high: string) =>
    onChange(low === '' ? '' : rpeTarget(Number(low), high === '' ? undefined : Number(high)));

  return (
    <View className="mb-3">
      <Text className="mb-2 text-sm font-medium text-text-secondary">{t(SESSION_EDITOR_KEYS.stepTarget)}</Text>
      <View
        className="flex-row flex-wrap gap-1.5"
        accessibilityRole="radiogroup"
        accessibilityLabel={t(SESSION_EDITOR_KEYS.stepTarget)}
        testID={id}
      >
        {ZONES.map((zone) => (
          <Chip
            key={zone}
            label={zoneTarget(zone)}
            shape="tile"
            role="radio"
            selected={panel === 'zone' && parsed.kind === 'zone' && parsed.zone === zone}
            onPress={() => {
              setPanel('zone');
              onChange(zoneTarget(zone));
            }}
            testID={`${id}-z${zone}`}
          />
        ))}
        <Chip
          label="%"
          accessibilityLabel={t(SESSION_EDITOR_KEYS.targetPercent)}
          shape="tile"
          role="radio"
          selected={panel === 'percent'}
          onPress={() => {
            if (panel === 'percent') return;
            setPanel('percent');
            writePercent(from, to, anchor);
          }}
          testID={`${id}-percent`}
        />
        <Chip
          label={t(SESSION_EDITOR_KEYS.targetRpe)}
          shape="tile"
          role="radio"
          selected={panel === 'rpe'}
          onPress={() => {
            if (panel === 'rpe') return;
            setPanel('rpe');
            writeRpe(rpeLow, rpeHigh);
          }}
          testID={`${id}-rpe`}
        />
        <Chip
          label={t(SESSION_EDITOR_KEYS.targetOther)}
          shape="tile"
          role="radio"
          selected={panel === 'typed'}
          onPress={() => setPanel('typed')}
          testID={`${id}-typed`}
        />
      </View>
      {panel === 'percent' ? (
        <View className="mt-3">
          <View className="flex-row gap-4">
            <View className="flex-1">
              <Input
                label={t(SESSION_EDITOR_KEYS.targetFrom)}
                value={from}
                onChangeText={(text) => {
                  const low = digits(text);
                  setFrom(low);
                  writePercent(low, to, anchor);
                }}
                keyboardType="number-pad"
                accessibilityLabel={t(SESSION_EDITOR_KEYS.targetFrom)}
                testID={`${id}-from`}
              />
            </View>
            <View className="flex-1">
              <Input
                label={t(SESSION_EDITOR_KEYS.targetTo)}
                value={to}
                onChangeText={(text) => {
                  const high = digits(text);
                  setTo(high);
                  writePercent(from, high, anchor);
                }}
                keyboardType="number-pad"
                accessibilityLabel={t(SESSION_EDITOR_KEYS.targetTo)}
                testID={`${id}-to`}
              />
            </View>
          </View>
          <RadioGroup
            layout="segmented"
            label={t(SESSION_EDITOR_KEYS.targetOf)}
            options={TARGET_ANCHORS.map((key) => ({ key, label: t(TARGET_ANCHOR_LABEL_KEY[key]) }))}
            value={anchor}
            onChange={(key) => {
              const of = TARGET_ANCHORS.find((candidate) => candidate === key) ?? null;
              setAnchor(of);
              writePercent(from, to, of);
            }}
            testID={`${id}-anchor`}
          />
        </View>
      ) : null}
      {panel === 'rpe' ? (
        <View className="mt-3 flex-row gap-4">
          <View className="flex-1">
            <Input
              label={t(SESSION_EDITOR_KEYS.targetRpeValue)}
              value={rpeLow}
              onChangeText={(text) => {
                const low = digits(text).slice(0, 2);
                setRpeLow(low);
                writeRpe(low, rpeHigh);
              }}
              keyboardType="number-pad"
              accessibilityLabel={t(SESSION_EDITOR_KEYS.targetRpeValue)}
              testID={`${id}-rpe-value`}
            />
          </View>
          <View className="flex-1">
            <Input
              label={t(SESSION_EDITOR_KEYS.targetTo)}
              value={rpeHigh}
              onChangeText={(text) => {
                const high = digits(text).slice(0, 2);
                setRpeHigh(high);
                writeRpe(rpeLow, high);
              }}
              keyboardType="number-pad"
              accessibilityLabel={t(SESSION_EDITOR_KEYS.targetTo)}
              testID={`${id}-rpe-to`}
            />
          </View>
        </View>
      ) : null}
      {panel === 'typed' ? (
        <View className="mt-3">
          <Input
            label={t(SESSION_EDITOR_KEYS.targetTyped)}
            value={target}
            onChangeText={onChange}
            accessibilityLabel={t(SESSION_EDITOR_KEYS.targetTyped)}
            autoCapitalize="none"
            testID={`${id}-text`}
          />
        </View>
      ) : null}
    </View>
  );
}

interface StepControlProps {
  icon: 'arrow-up' | 'arrow-down' | 'copy' | 'trash-2';
  label: string;
  disabled?: boolean;
  onPress: () => void;
  testID: string;
}

/** A 44pt icon action on a step, in the secondary ink; dimmed and inert where it would change nothing. */
const StepControl = forwardRef<View, StepControlProps>(function StepControl(
  { icon, label, disabled = false, onPress, testID },
  ref,
) {
  const colors = useThemeColors();
  return (
    <Pressable
      ref={ref}
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
});
