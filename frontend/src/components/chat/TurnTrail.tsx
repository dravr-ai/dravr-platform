// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A long turn's steps on the web — the live list with its timer and safe-to-leave note, and the line it folds into
// ABOUTME: Paints what @pierre/chat-utils' turn trail returns; every rule (labels, thresholds, the fold) lives there

import { useState } from 'react';
import { Check, ChevronRight } from 'lucide-react';
import clsx from 'clsx';
import {
  doneSteps,
  foldTurnTrail,
  TURN_STEP_KEYS,
  TURN_TRAIL_KEYS,
  turnCurrentLine,
  turnIndicator,
  type TranslatableText,
  type TurnStep,
  type TurnTrail,
} from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { useTurnElapsedMs } from '@pierre/ui-logic';

function useText(): (text: TranslatableText) => string {
  const { t } = useTranslation();
  return text => t(text.key, text.params);
}

function DoneStep({ step }: { step: TurnStep }) {
  const { t } = useTranslation();
  const text = useText();
  return (
    <li className="flex items-center gap-2 text-sm text-on-surface-variant" data-testid="turn-step-done">
      <Check className="h-3.5 w-3.5 shrink-0 text-primary" aria-hidden="true" />
      <span>{text(step.label)}</span>
      <span className="sr-only">{t(TURN_TRAIL_KEYS.stepDone)}</span>
    </li>
  );
}

interface LiveTurnProps {
  /** The turn's steps; `null` for a turn restored after a reload, whose steps are unknown. */
  trail: TurnTrail | null;
  /**
   * When the turn started (epoch ms): the restored turn's question time.
   * Defaults to the trail's own start, else to when this mounted.
   */
  startedAt?: number;
}

/** The elapsed time beside the current step, once the turn has run long enough. */
function Elapsed({ elapsed }: { elapsed: TranslatableText | null }) {
  const text = useText();
  if (elapsed === null) return null;
  // Outside the live region: a screen reader is told when the step changes,
  // not every second the clock moves.
  return (
    <span className="tabular-nums text-on-surface-variant" data-testid="turn-elapsed">
      <span aria-hidden="true">· </span>
      {text(elapsed)}
    </span>
  );
}

/**
 * The running turn, where its reply will be: the steps it has finished, each
 * checked, then the one it is on with a spinner and, after a few seconds, how
 * long it has run — and, on a long turn, that the athlete may leave.
 */
export function LiveTurnIndicator({ trail, startedAt }: LiveTurnProps) {
  const { t } = useTranslation();
  const text = useText();
  const indicator = turnIndicator(useTurnElapsedMs(startedAt ?? trail?.startedAt));
  const current: TranslatableText = trail ? turnCurrentLine(trail) : { key: TURN_STEP_KEYS.thinking };
  const done = trail ? doneSteps(trail) : [];
  return (
    <div className="flex flex-col gap-2" data-testid="turn-indicator">
      <ol className="flex flex-col gap-1" aria-label={t(TURN_TRAIL_KEYS.stepsLabel)}>
        {done.map(step => (
          <DoneStep key={step.id} step={step} />
        ))}
        <li className="flex items-center gap-2 text-sm text-on-surface" data-testid="turn-current-step">
          <span className="pierre-spinner h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          <span role="status" aria-live="polite">
            {text(current)}
          </span>
          <Elapsed elapsed={indicator.elapsed} />
        </li>
      </ol>
      {indicator.safeToLeave ? (
        <p className="text-xs leading-relaxed text-on-surface-variant" data-testid="turn-safe-to-leave">
          {text(indicator.safeToLeave)}
        </p>
      ) : null}
    </div>
  );
}

/** The one line under a reply's streaming words: what the turn is on, and for how long. */
export function LiveTurnLine({ trail, startedAt }: LiveTurnProps) {
  const text = useText();
  const indicator = turnIndicator(useTurnElapsedMs(startedAt ?? trail?.startedAt));
  const current: TranslatableText = trail ? turnCurrentLine(trail) : { key: TURN_STEP_KEYS.thinking };
  return (
    <div className="mt-1 flex items-center gap-2 text-xs text-on-surface-variant" data-testid="turn-current-step">
      <span className="pierre-spinner h-3 w-3 shrink-0" aria-hidden="true" />
      <span role="status" aria-live="polite">
        {text(current)}
      </span>
      <Elapsed elapsed={indicator.elapsed} />
    </div>
  );
}

/**
 * A finished turn, folded into one line above its reply — "Thought for
 * 7 min · 6 steps" — that opens into the steps it took. Nothing for a quick
 * answer that called no tool.
 */
export function FoldedTurnTrail({ trail }: { trail: TurnTrail }) {
  const { t } = useTranslation();
  const text = useText();
  const [open, setOpen] = useState(false);
  const folded = foldTurnTrail(trail);
  if (folded === null) return null;
  const line = t(folded.line.key, { ...folded.line.params, duration: text(folded.duration) });
  if (folded.steps.length === 0) {
    return (
      <p className="mb-1 text-xs text-on-surface-variant" data-testid="turn-trail-folded">
        {line}
      </p>
    );
  }
  return (
    <div className="mb-1" data-testid="turn-trail-folded">
      <button
        type="button"
        onClick={() => setOpen(value => !value)}
        aria-expanded={open}
        title={t(open ? TURN_TRAIL_KEYS.hideSteps : TURN_TRAIL_KEYS.showSteps)}
        className="flex items-center gap-1 rounded text-xs text-on-surface-variant transition-colors hover:text-on-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
      >
        <ChevronRight
          className={clsx('h-3.5 w-3.5 shrink-0 transition-transform', open && 'rotate-90')}
          aria-hidden="true"
        />
        <span>{line}</span>
      </button>
      {open ? (
        <ol
          className="mt-1 flex flex-col gap-1 border-l ghost-border pl-3"
          aria-label={t(TURN_TRAIL_KEYS.stepsLabel)}
          data-testid="turn-trail-steps"
        >
          {folded.steps.map(step => (
            <DoneStep key={step.id} step={step} />
          ))}
        </ol>
      ) : null}
    </div>
  );
}
