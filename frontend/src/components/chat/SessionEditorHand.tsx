// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The day editor's "Write" mode body — the session's minutes and intensity, or its steps and sets drawn as a profile (carnet#1714, #1727)
// ABOUTME: Holds only what the coach writes; it never waits on the family catalogue, and nothing here is checked against a family's evidence

import { useEffect, useId, useRef, useState } from 'react';
import { Minus, Plus, Repeat } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { PlanStep, SessionSport } from '@pierre/shared-types';
import {
  SERIES_MAX_REPEAT,
  SESSION_EDITOR_KEYS,
  addSeries,
  addStep,
  addStepToSeries,
  duplicateStep,
  handStatesDuration,
  moveStep,
  removeStep,
  seriesDefaults,
  setHand,
  setSeriesRepeat,
  setStepMeasureOf,
  stepBlocks,
  stepMoveTarget,
  updateStep,
  type HandSession,
} from '@pierre/ui-logic';
import { Button, IconButton, Input } from '../ui';
import { SessionEditorProfile } from './SessionEditorProfile';
import { StepCard, StepRow } from './SessionEditorStep';
import { digitsOf } from './sessionEditorFields';

interface SessionEditorHandProps {
  hand: HandSession;
  /** The family sport the session reads as: a new set's targets and a percent target's threshold start from it. */
  familySport: SessionSport | undefined;
  onChange: (hand: HandSession) => void;
}

/**
 * The session as the coach writes it. The minutes and intensity are the
 * coach's to state only while the session has no steps: with steps the
 * server writes the duration they total and the span their targets cover, so
 * the fields step aside (and keep their values for when the steps are gone).
 */
export function SessionEditorHand({ hand, familySport, onChange }: SessionEditorHandProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-5">
      {handStatesDuration(hand) && (
        <div className="grid grid-cols-2 gap-x-4">
          <Input
            label={t(SESSION_EDITOR_KEYS.duration)}
            type="text"
            inputMode="numeric"
            pattern="[0-9]*"
            autoComplete="off"
            className="font-mono"
            value={hand.duration_min === undefined ? '' : String(hand.duration_min)}
            onChange={(event) => onChange(setHand(hand, { duration_min: digitsOf(event.target.value) }))}
            data-testid="session-editor-duration"
          />
          <Input
            label={t(SESSION_EDITOR_KEYS.intensity)}
            value={hand.intensity}
            autoComplete="off"
            onChange={(event) => onChange(setHand(hand, { intensity: event.target.value }))}
            data-testid="session-editor-intensity"
          />
        </div>
      )}

      <StepsEditor hand={hand} familySport={familySport} onChange={onChange} />

      <p className="text-xs text-on-surface-variant" data-testid="session-editor-unchecked">
        {t(SESSION_EDITOR_KEYS.unchecked)}
      </p>
    </div>
  );
}

/**
 * The steps as blocks — a lone step, or a set under its × N — each step a
 * folded row until it is opened, one open at a time. A step just added opens
 * with its name field focused. Moving, copying or removing a step keeps the
 * keyboard where the coach was working: focus follows the moved step, and a
 * removed step hands it to the row now in its place, else to Add a step.
 */
function StepsEditor({
  hand,
  familySport,
  onChange,
}: {
  hand: HandSession;
  familySport: SessionSport | undefined;
  onChange: (hand: HandSession) => void;
}) {
  const { t } = useTranslation();
  const headingId = useId();
  const sectionRef = useRef<HTMLElement>(null);
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  // The control that takes focus once the steps have re-rendered, by its `data-focus-key`.
  const focusAfter = useRef<string | null>(null);
  const steps = hand.steps;
  const count = steps.length;
  const blocks = stepBlocks(steps);
  const open = openIndex !== null && openIndex < count ? openIndex : null;

  useEffect(() => {
    const key = focusAfter.current;
    if (key === null) return;
    focusAfter.current = null;
    sectionRef.current?.querySelector<HTMLElement>(`[data-focus-key="${key}"]`)?.focus();
  });

  /** Open the step at `index` and hand its name field the keyboard. */
  const openStep = (index: number, next: HandSession = hand) => {
    focusAfter.current = `label-${index}`;
    setOpenIndex(index);
    if (next !== hand) onChange(next);
  };

  const move = (index: number, delta: -1 | 1) => {
    const to = stepMoveTarget(steps, index, delta);
    if (to === null) return;
    const next = moveStep(hand, index, delta);
    // The same control on the step's new place, unless the move took it to an end where it is disabled.
    const spent = stepMoveTarget(next.steps, to, delta) === null;
    const control = delta === -1 ? (spent ? 'down' : 'up') : spent ? 'up' : 'down';
    focusAfter.current = `${control}-${to}`;
    setOpenIndex(to);
    onChange(next);
  };

  const remove = (index: number) => {
    const left = count - 1;
    focusAfter.current = left === 0 ? 'add' : `row-${Math.min(index, left - 1)}`;
    setOpenIndex(null);
    onChange(removeStep(hand, index));
  };

  const repeatSeries = (start: number, repeat: number) => {
    // A set taken down to one round dissolves into lone steps: its stepper goes, so the keyboard goes to its first row.
    if (repeat <= 1) {
      focusAfter.current = `row-${start}`;
      setOpenIndex(null);
    }
    onChange(setSeriesRepeat(hand, start, repeat));
  };

  const row = (index: number) => {
    const step = steps[index] as PlanStep;
    const number = index + 1;
    return (
      <li key={index} data-testid={`session-editor-step-${number}`}>
        {open === index ? (
          <StepCard
            step={step}
            index={index}
            familySport={familySport}
            canMoveUp={stepMoveTarget(steps, index, -1) !== null}
            canMoveDown={stepMoveTarget(steps, index, 1) !== null}
            onChange={(patch) => onChange(updateStep(hand, index, patch))}
            onMeasure={(measure) => onChange(setStepMeasureOf(hand, index, measure))}
            onMove={(delta) => move(index, delta)}
            onDuplicate={() => openStep(index + 1, duplicateStep(hand, index))}
            onRemove={() => remove(index)}
          />
        ) : (
          <StepRow step={step} index={index} onOpen={() => openStep(index)} />
        )}
      </li>
    );
  };

  return (
    <section ref={sectionRef} aria-labelledby={headingId} className="space-y-3" data-testid="session-editor-steps">
      <h3 id={headingId} className="text-sm font-medium text-on-surface-variant">
        {t(SESSION_EDITOR_KEYS.stepsHeading)}
      </h3>
      {count === 0 && (
        <p className="text-sm text-on-surface-variant" data-testid="session-editor-no-steps">
          {t(SESSION_EDITOR_KEYS.noSteps)}
        </p>
      )}
      {count > 0 && <SessionEditorProfile steps={steps} openIndex={open} onOpen={(index) => openStep(index)} />}
      {count > 0 && (
        <ol className="space-y-1">
          {blocks.map((block) => {
            if (block.kind === 'step') return row(block.index);
            const indices = Array.from({ length: block.end - block.start }, (_, at) => block.start + at);
            return (
              <li key={`series-${block.start}`} className="my-2 border-l-2 border-on-surface pl-3" data-testid={`session-editor-series-${block.start + 1}`}>
                <SeriesHeader
                  repeat={block.repeat}
                  onRepeat={(repeat) => repeatSeries(block.start, repeat)}
                  testId={`session-editor-series-${block.start + 1}`}
                />
                <ol className="space-y-1">{indices.map(row)}</ol>
                <Button
                  variant="tertiary"
                  size="sm"
                  className="mt-1"
                  onClick={() => openStep(block.end, addStepToSeries(hand, block.start))}
                  data-testid={`session-editor-series-${block.start + 1}-add`}
                >
                  <Plus className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
                  {t(SESSION_EDITOR_KEYS.addStepToSeries)}
                </Button>
              </li>
            );
          })}
        </ol>
      )}
      <div className="-ml-2 flex flex-wrap gap-x-4">
        <Button variant="tertiary" onClick={() => openStep(count, addStep(hand))} data-focus-key="add" data-testid="session-editor-add-step">
          <Plus className="mr-1.5 h-4 w-4" aria-hidden="true" />
          {t(SESSION_EDITOR_KEYS.addStep)}
        </Button>
        <Button
          variant="tertiary"
          onClick={() =>
            openStep(
              count,
              addSeries(
                hand,
                seriesDefaults(familySport, {
                  work: t(SESSION_EDITOR_KEYS.seriesWork),
                  recovery: t(SESSION_EDITOR_KEYS.seriesRecovery),
                }),
              ),
            )
          }
          data-testid="session-editor-add-series"
        >
          <Repeat className="mr-1.5 h-4 w-4" aria-hidden="true" />
          {t(SESSION_EDITOR_KEYS.addSeries)}
        </Button>
      </div>
    </section>
  );
}

/**
 * A set's heading — "Set", and its − × N + stepper. One round fewer than two
 * dissolves the set into lone steps; the count stops at the set's ceiling.
 */
function SeriesHeader({
  repeat,
  onRepeat,
  testId,
}: {
  repeat: number;
  onRepeat: (repeat: number) => void;
  testId: string;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex min-h-10 items-center justify-between gap-3">
      <span className="text-sm font-semibold text-on-surface-variant">{t(SESSION_EDITOR_KEYS.series)}</span>
      <div role="group" aria-label={t(SESSION_EDITOR_KEYS.seriesRepeat)} className="flex items-center gap-1">
        <IconButton
          variant="tonal"
          size="md"
          aria-label={t(SESSION_EDITOR_KEYS.seriesFewer)}
          title={t(SESSION_EDITOR_KEYS.seriesFewer)}
          onClick={() => onRepeat(repeat - 1)}
          data-testid={`${testId}-fewer`}
        >
          <Minus className="h-4 w-4" aria-hidden="true" />
        </IconButton>
        <output aria-live="polite" className="w-10 text-center font-mono text-sm text-on-surface" data-testid={`${testId}-repeat`}>
          × {repeat}
        </output>
        <IconButton
          variant="tonal"
          size="md"
          aria-label={t(SESSION_EDITOR_KEYS.seriesMore)}
          title={t(SESSION_EDITOR_KEYS.seriesMore)}
          disabled={repeat >= SERIES_MAX_REPEAT}
          onClick={() => onRepeat(repeat + 1)}
          data-testid={`${testId}-more`}
        >
          <Plus className="h-4 w-4" aria-hidden="true" />
        </IconButton>
      </div>
    </div>
  );
}
