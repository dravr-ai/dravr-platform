// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One step of a hand-written session — a collapsed row that opens it, and the open card: name, time or distance, target (carnet#1727)
// ABOUTME: Targets are chips (Z1–Z7, %, RPE, Other) composed in the server's grammar; whether a typed one resolves is the server's to say

import { useId, useState, type ReactNode } from 'react';
import { ArrowDown, ArrowUp, Copy, Minus, Plus, Trash2 } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { PlanStep, SessionSport } from '@pierre/shared-types';
import {
  MAX_TARGET_ZONE,
  SESSION_EDITOR_KEYS,
  TARGET_ANCHORS,
  TARGET_ANCHOR_LABEL_KEY,
  defaultAnchor,
  formatStepDuration,
  parseStepDuration,
  parseTarget,
  percentTarget,
  rpeTarget,
  stepMeasure,
  zoneTarget,
  type StepMeasure,
  type StepTarget,
  type TargetAnchor,
  type Translate,
} from '@pierre/ui-logic';
import { Chip, IconButton, Input, SegmentedControl, Select } from '../ui';
import { digitsOf } from './sessionEditorFields';

/** How far the −/+ buttons beside a step's time move it. */
const NUDGE_SECONDS = 15;

/** The zones a target chip offers, Z1 to the grammar's highest. */
const ZONES = Array.from({ length: MAX_TARGET_ZONE }, (_, at) => at + 1);

/** A small grey tag on a collapsed row: a time, a distance, a target. */
const TAG = 'inline-flex h-6 items-center rounded-md bg-surface-container-low px-1.5 font-mono text-xs text-on-surface';

/** A step's distance as the row tags it, in the coach's notation: `400 m`, the unit read from the catalogue. */
function formatDistance(t: Translate, meters: number, language: string): string {
  const value = new Intl.NumberFormat(language, { maximumFractionDigits: 0 }).format(meters);
  return t(SESSION_EDITOR_KEYS.distanceTag, { value });
}

interface StepRowProps {
  step: PlanStep;
  index: number;
  onOpen: () => void;
}

/**
 * A step folded to one line: its number (a picture; the button names it),
 * its name, and its tags — the time, or the distance and its estimated time,
 * then the target as written. The whole row is the button that opens it.
 */
export function StepRow({ step, index, onOpen }: StepRowProps) {
  const { t, language } = useTranslation();
  const number = index + 1;
  const target = step.target_zone.trim();
  return (
    <button
      type="button"
      aria-label={t(SESSION_EDITOR_KEYS.editStep, { number, label: step.label })}
      onClick={onOpen}
      data-focus-key={`row-${index}`}
      data-testid={`session-editor-step-${number}-row`}
      className="grid min-h-12 w-full grid-cols-[1.25rem_minmax(0,1fr)_auto] items-center gap-2.5 rounded-lg pr-1 text-left transition-colors duration-base hover:bg-surface-container-low focus-ring-visible lg:min-h-11"
    >
      <span aria-hidden="true" className="text-right font-mono text-xs text-outline">
        {number}
      </span>
      <span className="truncate text-base font-medium text-on-surface">{step.label}</span>
      <span className="flex shrink-0 gap-1">
        {step.distance_meters !== undefined && <span className={TAG}>{formatDistance(t, step.distance_meters, language)}</span>}
        <span className={TAG}>{formatStepDuration(step.duration_seconds)}</span>
        {target && <span className={TAG}>{target}</span>}
      </span>
    </button>
  );
}

interface StepCardProps {
  step: PlanStep;
  index: number;
  /** The family sport the session reads as, for the anchor a percent target starts on. */
  familySport: SessionSport | undefined;
  /** Whether each move would change anything: a move that would not is disabled. */
  canMoveUp: boolean;
  canMoveDown: boolean;
  onChange: (patch: Partial<PlanStep>) => void;
  onMeasure: (measure: StepMeasure) => void;
  onMove: (delta: -1 | 1) => void;
  onDuplicate: () => void;
  onRemove: () => void;
}

/**
 * The step open for editing, edged in `primary`: its name with the step's
 * moves, copy and remove beside it; whether it is measured by time or by a
 * distance (with the time it is estimated to take, which the session's total
 * is written from); then its target.
 */
export function StepCard({
  step,
  index,
  familySport,
  canMoveUp,
  canMoveDown,
  onChange,
  onMeasure,
  onMove,
  onDuplicate,
  onRemove,
}: StepCardProps) {
  const { t } = useTranslation();
  const number = index + 1;
  const testId = `session-editor-step-${number}`;
  const measure = stepMeasure(step);
  return (
    <div className="space-y-4 rounded-xl border border-primary bg-surface-container-lowest px-3 pb-4 pt-2" data-testid={`${testId}-card`}>
      {/* The step's number, a picture (each action names it), and what can be done to the step. */}
      <div className="-mr-1 -mt-1 flex items-center justify-between gap-2">
        <span aria-hidden="true" className="font-mono text-xs text-outline">
          {number}
        </span>
        <div className="flex shrink-0 items-center">
          <StepAction label={t(SESSION_EDITOR_KEYS.moveStepUp, { number })} disabled={!canMoveUp} onClick={() => onMove(-1)} focusKey={`up-${index}`}>
            <ArrowUp className="h-4 w-4" aria-hidden="true" />
          </StepAction>
          <StepAction label={t(SESSION_EDITOR_KEYS.moveStepDown, { number })} disabled={!canMoveDown} onClick={() => onMove(1)} focusKey={`down-${index}`}>
            <ArrowDown className="h-4 w-4" aria-hidden="true" />
          </StepAction>
          <StepAction label={t(SESSION_EDITOR_KEYS.duplicateStep, { number })} onClick={onDuplicate} focusKey={`duplicate-${index}`}>
            <Copy className="h-4 w-4" aria-hidden="true" />
          </StepAction>
          <StepAction label={t(SESSION_EDITOR_KEYS.removeStep, { number })} onClick={onRemove} focusKey={`remove-${index}`}>
            <Trash2 className="h-4 w-4" aria-hidden="true" />
          </StepAction>
        </div>
      </div>

      <Input
        label={t(SESSION_EDITOR_KEYS.stepLabel)}
        value={step.label}
        autoComplete="off"
        onChange={(event) => onChange({ label: event.target.value })}
        data-focus-key={`label-${index}`}
        data-testid={`${testId}-label`}
      />

      <div className="flex flex-wrap items-end gap-x-4 gap-y-3">
        <SegmentedControl
          value={measure}
          onChange={onMeasure}
          aria-label={t(SESSION_EDITOR_KEYS.measure)}
          className="mb-0.5"
          data-testid={`${testId}-measure`}
          options={[
            { value: 'time', label: t(SESSION_EDITOR_KEYS.measureTime) },
            { value: 'distance', label: t(SESSION_EDITOR_KEYS.measureDistance) },
          ]}
        />
        {measure === 'distance' && (
          <div className="w-28">
            <Input
              label={t(SESSION_EDITOR_KEYS.stepDistance)}
              type="text"
              inputMode="numeric"
              pattern="[0-9]*"
              autoComplete="off"
              className="font-mono"
              value={step.distance_meters ? String(step.distance_meters) : ''}
              onChange={(event) => onChange({ distance_meters: digitsOf(event.target.value) ?? 0 })}
              data-testid={`${testId}-distance`}
            />
          </div>
        )}
        <StepDurationField
          label={t(measure === 'distance' ? SESSION_EDITOR_KEYS.stepEstimate : SESSION_EDITOR_KEYS.stepDuration)}
          seconds={step.duration_seconds}
          nudge={measure === 'time'}
          onChange={(duration_seconds) => onChange({ duration_seconds })}
          data-testid={`${testId}-duration`}
        />
      </div>

      <StepTargetPicker
        target={step.target_zone}
        familySport={familySport}
        onChange={(target_zone) => onChange({ target_zone })}
        testId={`${testId}-target`}
      />
    </div>
  );
}

/** One of an open step's header actions: an icon button named by what it does to the numbered step. */
function StepAction({
  label,
  disabled,
  onClick,
  focusKey,
  children,
}: {
  label: string;
  disabled?: boolean;
  onClick: () => void;
  focusKey: string;
  children: ReactNode;
}) {
  return (
    <IconButton size="sm" aria-label={label} title={label} disabled={disabled} onClick={onClick} data-focus-key={focusKey}>
      {children}
    </IconButton>
  );
}

/**
 * A step's duration in `m:ss`. While the field has focus it shows what the
 * coach typed, and each reading that parses is applied as it is typed; on
 * blur it shows the duration the step holds, written the one way. A step
 * measured by time has −/+ buttons beside it that move it by 15 s, never
 * below zero (where Save says the step needs a time).
 */
function StepDurationField({
  label,
  seconds,
  nudge,
  onChange,
  'data-testid': testId,
}: {
  label: string;
  seconds: number;
  /** Whether the −/+ 15 s buttons flank the field. */
  nudge: boolean;
  onChange: (seconds: number) => void;
  'data-testid': string;
}) {
  const { t } = useTranslation();
  const [typing, setTyping] = useState<string | null>(null);
  const field = (
    <div className={nudge ? 'w-20' : 'w-40'}>
      <Input
        label={label}
        type="text"
        autoComplete="off"
        spellCheck={false}
        className={nudge ? 'text-center font-mono' : 'font-mono'}
        value={typing ?? formatStepDuration(seconds)}
        onFocus={() => setTyping(formatStepDuration(seconds))}
        onBlur={() => setTyping(null)}
        onChange={(event) => {
          setTyping(event.target.value);
          const parsed = parseStepDuration(event.target.value);
          if (parsed !== undefined) onChange(parsed);
        }}
        data-testid={testId}
      />
    </div>
  );
  if (!nudge) return field;
  return (
    <div className="flex items-end gap-1">
      <IconButton
        variant="tonal"
        aria-label={t(SESSION_EDITOR_KEYS.durationLess)}
        title={t(SESSION_EDITOR_KEYS.durationLess)}
        disabled={seconds <= 0}
        onClick={() => onChange(Math.max(0, seconds - NUDGE_SECONDS))}
        className="mb-0.5"
        data-testid={`${testId}-less`}
      >
        <Minus className="h-4 w-4" aria-hidden="true" />
      </IconButton>
      {field}
      <IconButton
        variant="tonal"
        aria-label={t(SESSION_EDITOR_KEYS.durationMore)}
        title={t(SESSION_EDITOR_KEYS.durationMore)}
        onClick={() => onChange(seconds + NUDGE_SECONDS)}
        className="mb-0.5"
        data-testid={`${testId}-more`}
      >
        <Plus className="h-4 w-4" aria-hidden="true" />
      </IconButton>
    </div>
  );
}

/** A number field's text as the coach typed it, from a figure the target holds. */
function figureText(value: number | undefined): string {
  return value === undefined ? '' : String(value);
}

/**
 * The target chips, picked by what the target reads as: a zone writes `Z4`
 * at once; "%" reveals From / To and the threshold it is of and writes
 * `88-93% FTP`; "RPE" reveals the 1–10 value(s) and writes `RPE 7`; "Other"
 * keeps the target as written (an agent's "sweet spot" opens there). Picking
 * % or RPE over a target of another kind clears it until the coach states
 * the figures, and the footer says the step still needs one.
 */
function StepTargetPicker({
  target,
  familySport,
  onChange,
  testId,
}: {
  target: string;
  familySport: SessionSport | undefined;
  onChange: (target: string) => void;
  testId: string;
}) {
  const { t } = useTranslation();
  const labelId = useId();
  const parsed = parseTarget(target);
  // Which chip is picked: what the target reads as, until the coach picks
  // a kind whose figures are not typed yet.
  const [kind, setKind] = useState<StepTarget['kind']>(parsed.kind);
  const [low, setLow] = useState(() =>
    figureText(parsed.kind === 'percent' || parsed.kind === 'rpe' ? parsed.low : undefined),
  );
  const [high, setHigh] = useState(() =>
    figureText(parsed.kind === 'percent' || parsed.kind === 'rpe' ? (parsed.high === parsed.low ? undefined : parsed.high) : undefined),
  );
  const [anchor, setAnchor] = useState<TargetAnchor>(
    parsed.kind === 'percent' && parsed.anchor ? parsed.anchor : defaultAnchor(familySport),
  );

  /** Write the band the figures state, or nothing until the first one is typed. */
  const writeBand = (as: 'percent' | 'rpe', from: string, to: string, of: TargetAnchor) => {
    const a = digitsOf(from);
    if (a === undefined) {
      onChange('');
      return;
    }
    const b = digitsOf(to) ?? a;
    onChange(as === 'percent' ? percentTarget(a, b, of) : rpeTarget(a, b));
  };

  const pickBand = (as: 'percent' | 'rpe') => {
    setKind(as);
    if (parsed.kind === as) return;
    setLow('');
    setHigh('');
    onChange('');
  };

  return (
    <div className="space-y-3">
      <div>
        <span id={labelId} className="mb-2 block text-sm font-medium text-on-surface-variant">
          {t(SESSION_EDITOR_KEYS.stepTarget)}
        </span>
        <div role="group" aria-labelledby={labelId} className="flex flex-wrap gap-1.5" data-testid={testId}>
          {ZONES.map((zone) => (
            <Chip
              key={zone}
              shape="code"
              className="font-mono"
              selected={kind === 'zone' && parsed.kind === 'zone' && parsed.zone === zone}
              onClick={() => {
                setKind('zone');
                onChange(zoneTarget(zone));
              }}
            >
              {zoneTarget(zone)}
            </Chip>
          ))}
          <Chip shape="code" className="font-mono" selected={kind === 'percent'} aria-label={t(SESSION_EDITOR_KEYS.targetPercent)} onClick={() => pickBand('percent')}>
            %
          </Chip>
          <Chip shape="code" className="font-mono" selected={kind === 'rpe'} onClick={() => pickBand('rpe')}>
            {t(SESSION_EDITOR_KEYS.targetRpe)}
          </Chip>
          <Chip shape="code" selected={kind === 'typed'} onClick={() => setKind('typed')}>
            {t(SESSION_EDITOR_KEYS.targetOther)}
          </Chip>
        </div>
      </div>

      {kind === 'percent' && (
        <div className="grid grid-cols-[4.5rem_4.5rem_minmax(0,1fr)] items-end gap-3">
          <Input
            label={t(SESSION_EDITOR_KEYS.targetFrom)}
            inputMode="numeric"
            pattern="[0-9]*"
            maxLength={3}
            autoComplete="off"
            className="font-mono"
            value={low}
            onChange={(event) => {
              setLow(event.target.value);
              writeBand('percent', event.target.value, high, anchor);
            }}
            data-testid={`${testId}-from`}
          />
          <Input
            label={t(SESSION_EDITOR_KEYS.targetTo)}
            inputMode="numeric"
            pattern="[0-9]*"
            maxLength={3}
            autoComplete="off"
            className="font-mono"
            value={high}
            onChange={(event) => {
              setHigh(event.target.value);
              writeBand('percent', low, event.target.value, anchor);
            }}
            data-testid={`${testId}-to`}
          />
          <Select
            label={t(SESSION_EDITOR_KEYS.targetOf)}
            value={anchor}
            options={TARGET_ANCHORS.map((option) => ({ value: option, label: t(TARGET_ANCHOR_LABEL_KEY[option]) }))}
            onChange={(event) => {
              const of = event.target.value as TargetAnchor;
              setAnchor(of);
              writeBand('percent', low, high, of);
            }}
            data-testid={`${testId}-anchor`}
          />
        </div>
      )}

      {kind === 'rpe' && (
        <div className="grid grid-cols-[6rem_4.5rem] items-end gap-3">
          <Input
            label={t(SESSION_EDITOR_KEYS.targetRpeValue)}
            inputMode="numeric"
            pattern="[0-9]*"
            maxLength={2}
            autoComplete="off"
            className="font-mono"
            value={low}
            onChange={(event) => {
              setLow(event.target.value);
              writeBand('rpe', event.target.value, high, anchor);
            }}
            data-testid={`${testId}-rpe`}
          />
          <Input
            label={t(SESSION_EDITOR_KEYS.targetTo)}
            inputMode="numeric"
            pattern="[0-9]*"
            maxLength={2}
            autoComplete="off"
            className="font-mono"
            value={high}
            onChange={(event) => {
              setHigh(event.target.value);
              writeBand('rpe', low, event.target.value, anchor);
            }}
            data-testid={`${testId}-rpe-to`}
          />
        </div>
      )}

      {kind === 'typed' && (
        <Input
          label={t(SESSION_EDITOR_KEYS.targetTyped)}
          value={target}
          autoComplete="off"
          onChange={(event) => onChange(event.target.value)}
          data-testid={`${testId}-typed`}
        />
      )}
    </div>
  );
}
