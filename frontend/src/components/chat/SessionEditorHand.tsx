// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The day editor's "This session" mode — the day's own session edited in place: rest, title, sport, duration, intensity and steps (carnet#1714)
// ABOUTME: Holds only what the coach writes; it never waits on the family catalogue, and nothing here is checked against a family's evidence

import { useEffect, useId, useRef, useState } from 'react';
import { ArrowDown, ArrowUp, Plus, Trash2 } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { PlanStep } from '@pierre/shared-types';
import {
  SESSION_EDITOR_KEYS,
  addStep,
  formatStepDuration,
  handStatesDuration,
  moveStep,
  parseStepDuration,
  removeStep,
  setHand,
  sportLabel,
  updateStep,
  type HandSession,
} from '@pierre/ui-logic';
import { Button, IconButton, Input, Select, Switch } from '../ui';

interface SessionEditorHandProps {
  hand: HandSession;
  /** The sports offered, the day's own word first; held still while the coach picks among them. */
  sportOptions: readonly string[];
  /**
   * Whether the Rest day switch is offered: not on a session the coach adds,
   * since a rest entry beside sessions is the agent's to write (carnet#1716).
   */
  restOffered?: boolean;
  onChange: (hand: HandSession) => void;
}

/** A field's value as digits, or `undefined` once it is emptied. */
function digitsOf(text: string): number | undefined {
  const digits = text.replace(/\D/g, '');
  return digits === '' ? undefined : Number(digits);
}

/**
 * The session as the day holds it, every part editable. A rest day hides the
 * fields and keeps what was typed, so switching rest off brings it back. The
 * minutes are the coach's to state only while the session has no steps: with
 * steps the server writes the duration they total, so the field steps aside
 * (and keeps its value for when the steps are gone).
 */
export function SessionEditorHand({ hand, sportOptions, restOffered = true, onChange }: SessionEditorHandProps) {
  const { t } = useTranslation();
  const restId = useId();
  const statesDuration = handStatesDuration(hand);
  return (
    <div className="space-y-5">
      {restOffered && (
        <div className="flex min-h-11 items-center justify-between gap-3">
          <span id={restId} className="text-sm font-medium text-on-surface">
            {t(SESSION_EDITOR_KEYS.restDay)}
          </span>
          <Switch
            checked={hand.rest}
            onChange={(rest) => onChange(setHand(hand, { rest }))}
            aria-labelledby={restId}
            data-testid="session-editor-rest"
          />
        </div>
      )}

      {!hand.rest && (
        <>
          <Input
            label={t(SESSION_EDITOR_KEYS.workout)}
            value={hand.workout}
            autoComplete="off"
            onChange={(event) => onChange(setHand(hand, { workout: event.target.value }))}
            data-testid="session-editor-workout"
          />
          <div className={`grid grid-cols-2 gap-x-4 gap-y-5 ${statesDuration ? 'sm:grid-cols-3' : ''}`}>
            <Select
              label={t(SESSION_EDITOR_KEYS.sport)}
              value={hand.sport}
              options={sportOptions.map((sport) => ({ value: sport, label: sportLabel(t, sport) }))}
              onChange={(event) => onChange(setHand(hand, { sport: event.target.value }))}
              data-testid="session-editor-sport"
            />
            {statesDuration && (
              <Input
                label={t(SESSION_EDITOR_KEYS.duration)}
                type="text"
                inputMode="numeric"
                pattern="[0-9]*"
                autoComplete="off"
                value={hand.duration_min === undefined ? '' : String(hand.duration_min)}
                onChange={(event) => onChange(setHand(hand, { duration_min: digitsOf(event.target.value) }))}
                data-testid="session-editor-duration"
              />
            )}
            <div className={statesDuration ? 'col-span-2 sm:col-span-1' : undefined}>
              <Input
                label={t(SESSION_EDITOR_KEYS.intensity)}
                value={hand.intensity}
                autoComplete="off"
                onChange={(event) => onChange(setHand(hand, { intensity: event.target.value }))}
                data-testid="session-editor-intensity"
              />
            </div>
          </div>

          <StepsEditor hand={hand} onChange={onChange} />

          <p className="text-xs text-on-surface-variant" data-testid="session-editor-unchecked">
            {t(SESSION_EDITOR_KEYS.unchecked)}
          </p>
        </>
      )}
    </div>
  );
}

/**
 * The steps in order, one numbered row each, then "Add a step". Moving or
 * removing a step keeps the keyboard where the coach was working: focus
 * follows the moved step, and a removed step hands it to the row now in its
 * place, else to Add a step.
 */
function StepsEditor({ hand, onChange }: { hand: HandSession; onChange: (hand: HandSession) => void }) {
  const { t } = useTranslation();
  const headingId = useId();
  const sectionRef = useRef<HTMLElement>(null);
  // The control that takes focus once the steps have re-rendered, by its `data-focus-key`.
  const focusAfter = useRef<string | null>(null);
  const count = hand.steps.length;

  useEffect(() => {
    const key = focusAfter.current;
    if (key === null) return;
    focusAfter.current = null;
    sectionRef.current?.querySelector<HTMLElement>(`[data-focus-key="${key}"]`)?.focus();
  }, [hand.steps]);

  const move = (index: number, delta: -1 | 1) => {
    const to = index + delta;
    // The same control on the step's new row, unless the move took it to an end where it is disabled.
    const atEnd = delta === -1 ? to === 0 : to === count - 1;
    const control = delta === -1 ? (atEnd ? 'down' : 'up') : atEnd ? 'up' : 'down';
    focusAfter.current = `${control}-${to}`;
    onChange(moveStep(hand, index, delta));
  };

  const remove = (index: number) => {
    const left = count - 1;
    focusAfter.current = left === 0 ? 'add' : `remove-${Math.min(index, left - 1)}`;
    onChange(removeStep(hand, index));
  };

  const add = () => {
    focusAfter.current = `label-${count}`;
    onChange(addStep(hand));
  };

  return (
    <section ref={sectionRef} aria-labelledby={headingId} data-testid="session-editor-steps">
      <h3 id={headingId} className="mb-2 text-sm font-medium text-on-surface-variant">
        {t(SESSION_EDITOR_KEYS.stepsHeading)}
      </h3>
      {count === 0 ? (
        <p className="text-sm text-on-surface-variant" data-testid="session-editor-no-steps">
          {t(SESSION_EDITOR_KEYS.noSteps)}
        </p>
      ) : (
        <ol className="border-t ghost-border-faint">
          {hand.steps.map((step, index) => (
            <StepRow
              // The row is its place in the list: a field's typed text lives only while it has focus,
              // and a move or a remove is pressed on a button, which takes that focus first.
              key={index}
              step={step}
              index={index}
              count={count}
              onChange={(patch) => onChange(updateStep(hand, index, patch))}
              onMove={(delta) => move(index, delta)}
              onRemove={() => remove(index)}
            />
          ))}
        </ol>
      )}
      <Button
        variant="tertiary"
        className="-ml-2 mt-2"
        onClick={add}
        data-focus-key="add"
        data-testid="session-editor-add-step"
      >
        <Plus className="mr-1.5 h-4 w-4" aria-hidden="true" />
        {t(SESSION_EDITOR_KEYS.addStep)}
      </Button>
    </section>
  );
}

interface StepRowProps {
  step: PlanStep;
  index: number;
  count: number;
  onChange: (patch: Partial<PlanStep>) => void;
  onMove: (delta: -1 | 1) => void;
  onRemove: () => void;
}

/**
 * One step: its number, then the label with the row's move and remove
 * buttons, then its duration, target and repeat. The list's own numbering
 * tells a reader which step a field is in; the drawn number is its picture.
 */
function StepRow({ step, index, count, onChange, onMove, onRemove }: StepRowProps) {
  const { t } = useTranslation();
  const number = index + 1;
  const testId = `session-editor-step-${number}`;
  return (
    <li className="flex gap-3 border-b ghost-border-faint py-3" data-testid={testId}>
      <span aria-hidden="true" className="w-4 shrink-0 pt-[34px] text-right font-mono text-xs text-outline">
        {number}
      </span>
      <div className="min-w-0 flex-1 space-y-3">
        <div className="flex items-end gap-2">
          <div className="min-w-0 flex-1">
            <Input
              label={t(SESSION_EDITOR_KEYS.stepLabel)}
              value={step.label}
              autoComplete="off"
              onChange={(event) => onChange({ label: event.target.value })}
              data-focus-key={`label-${index}`}
              data-testid={`${testId}-label`}
            />
          </div>
          <div className="flex shrink-0 items-center pb-0.5">
            <IconButton
              size="sm"
              aria-label={t(SESSION_EDITOR_KEYS.moveStepUp, { number })}
              title={t(SESSION_EDITOR_KEYS.moveStepUp, { number })}
              disabled={index === 0}
              onClick={() => onMove(-1)}
              data-focus-key={`up-${index}`}
            >
              <ArrowUp className="h-4 w-4" aria-hidden="true" />
            </IconButton>
            <IconButton
              size="sm"
              aria-label={t(SESSION_EDITOR_KEYS.moveStepDown, { number })}
              title={t(SESSION_EDITOR_KEYS.moveStepDown, { number })}
              disabled={index === count - 1}
              onClick={() => onMove(1)}
              data-focus-key={`down-${index}`}
            >
              <ArrowDown className="h-4 w-4" aria-hidden="true" />
            </IconButton>
            <IconButton
              size="sm"
              aria-label={t(SESSION_EDITOR_KEYS.removeStep, { number })}
              title={t(SESSION_EDITOR_KEYS.removeStep, { number })}
              onClick={onRemove}
              data-focus-key={`remove-${index}`}
            >
              <Trash2 className="h-4 w-4" aria-hidden="true" />
            </IconButton>
          </div>
        </div>
        <div className="grid grid-cols-3 gap-x-3">
          <StepDurationField
            seconds={step.duration_seconds}
            onChange={(duration_seconds) => onChange({ duration_seconds })}
            data-testid={`${testId}-duration`}
          />
          <Input
            label={t(SESSION_EDITOR_KEYS.stepTarget)}
            value={step.target_zone}
            autoComplete="off"
            onChange={(event) => onChange({ target_zone: event.target.value })}
            data-testid={`${testId}-target`}
          />
          <Input
            label={t(SESSION_EDITOR_KEYS.stepRepeat)}
            type="text"
            inputMode="numeric"
            pattern="[0-9]*"
            autoComplete="off"
            value={step.repeat === undefined ? '' : String(step.repeat)}
            onChange={(event) => onChange({ repeat: digitsOf(event.target.value) })}
            data-testid={`${testId}-repeat`}
          />
        </div>
      </div>
    </li>
  );
}

/**
 * A step's duration in `m:ss`. While the field has focus it shows what the
 * coach typed, and each reading that parses is applied as it is typed; on
 * blur it shows the duration the step holds, written the one way.
 */
function StepDurationField({
  seconds,
  onChange,
  'data-testid': testId,
}: {
  seconds: number;
  onChange: (seconds: number) => void;
  'data-testid': string;
}) {
  const { t } = useTranslation();
  const [typing, setTyping] = useState<string | null>(null);
  return (
    <Input
      label={t(SESSION_EDITOR_KEYS.stepDuration)}
      type="text"
      autoComplete="off"
      spellCheck={false}
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
  );
}
