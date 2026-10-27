// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The live turn indicator's timer and note thresholds, and the folded line a finished turn leaves above its reply
// ABOUTME: Renders the components alone over hand-built trails from @pierre/chat-utils

import { describe, it, expect, afterEach, vi } from 'vitest';
import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { i18n } from '@pierre/i18n';
import {
  reduceTurnTrail,
  startTurnTrail,
  TURN_ELAPSED_AFTER_MS,
  TURN_SAFE_TO_LEAVE_AFTER_MS,
  TURN_STEP_KEYS,
  TURN_TRAIL_KEYS,
  type TurnTrail,
} from '@pierre/chat-utils';
import { FoldedTurnTrail, LiveTurnIndicator, LiveTurnLine } from '../TurnTrail';

function withTool(trail: TurnTrail, id: string, title: string, status: string): TurnTrail {
  return reduceTurnTrail(trail, { type: 'progress', progress: { kind: 'tool', id, title, status } });
}

describe('LiveTurnIndicator', () => {
  afterEach(() => vi.useRealTimers());

  it('shows only the current step for a turn that just started', () => {
    render(<LiveTurnIndicator trail={startTurnTrail(Date.now())} />);
    expect(screen.getByTestId('turn-current-step')).toHaveTextContent(i18n.t(TURN_STEP_KEYS.thinking));
    expect(screen.queryByTestId('turn-elapsed')).not.toBeInTheDocument();
    expect(screen.queryByTestId('turn-safe-to-leave')).not.toBeInTheDocument();
  });

  it('adds the timer after ten seconds and the safe-to-leave note after forty-five, as the clock ticks', () => {
    vi.useFakeTimers();
    render(<LiveTurnIndicator trail={startTurnTrail(Date.now())} />);

    act(() => {
      vi.advanceTimersByTime(TURN_ELAPSED_AFTER_MS);
    });
    expect(screen.getByTestId('turn-elapsed')).toHaveTextContent(
      i18n.t(TURN_TRAIL_KEYS.elapsedSeconds, { seconds: 10 }),
    );
    expect(screen.queryByTestId('turn-safe-to-leave')).not.toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(TURN_SAFE_TO_LEAVE_AFTER_MS - TURN_ELAPSED_AFTER_MS + 89_000);
    });
    expect(screen.getByTestId('turn-elapsed')).toHaveTextContent(
      i18n.t(TURN_TRAIL_KEYS.elapsedMinutes, { minutes: 2, seconds: 14 }),
    );
    expect(screen.getByTestId('turn-safe-to-leave')).toHaveTextContent(i18n.t(TURN_TRAIL_KEYS.safeToLeave));
  });

  it('keeps the ticking timer out of the live region a screen reader announces', () => {
    render(<LiveTurnIndicator trail={startTurnTrail(Date.now() - 20_000)} />);
    const status = screen.getByRole('status');
    expect(status).not.toContainElement(screen.getByTestId('turn-elapsed'));
  });

  it('checks finished steps and spins on the running one', () => {
    let trail = withTool(startTurnTrail(Date.now()), 'c1', 'get_training_plan', 'Completed');
    trail = withTool(trail, 'c2', 'list_workout_templates', 'InProgress');
    render(<LiveTurnIndicator trail={trail} />);
    const done = screen.getAllByTestId('turn-step-done');
    expect(done).toHaveLength(1);
    expect(done[0]).toHaveTextContent(i18n.t(TURN_STEP_KEYS.readPlan));
    expect(screen.getByTestId('turn-current-step')).toHaveTextContent(i18n.t(TURN_STEP_KEYS.pickWorkouts));
  });

  it('times a restored turn from when its question was asked, with no steps', () => {
    render(<LiveTurnIndicator trail={null} startedAt={Date.now() - 134_000} />);
    expect(screen.getByTestId('turn-elapsed')).toHaveTextContent(
      i18n.t(TURN_TRAIL_KEYS.elapsedMinutes, { minutes: 2, seconds: 14 }),
    );
    expect(screen.queryByTestId('turn-step-done')).not.toBeInTheDocument();
  });
});

describe('LiveTurnLine', () => {
  it('names the current step under the streaming words', () => {
    const trail = reduceTurnTrail(startTurnTrail(Date.now()), { type: 'prose' });
    render(<LiveTurnLine trail={trail} />);
    expect(screen.getByTestId('turn-current-step')).toHaveTextContent(i18n.t(TURN_STEP_KEYS.writing));
  });
});

describe('FoldedTurnTrail', () => {
  const start = 1_000_000;

  it('draws nothing for a quick answer that called no tool', () => {
    const trail = reduceTurnTrail(startTurnTrail(start), { type: 'finished', at: start + 2_000 });
    const { container } = render(<FoldedTurnTrail trail={trail} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('folds a long turn into its duration and steps, and opens into them', async () => {
    let trail = withTool(startTurnTrail(start), 'c1', 'recommend_plan_flavour', 'Completed');
    trail = withTool(trail, 'c2', 'save_training_plan', 'Completed');
    trail = reduceTurnTrail(trail, { type: 'finished', at: start + 420_000 });
    render(<FoldedTurnTrail trail={trail} />);

    const duration = i18n.t(TURN_TRAIL_KEYS.elapsedMinutes, { minutes: 7, seconds: 0 });
    const toggle = screen.getByRole('button', {
      name: i18n.t(TURN_TRAIL_KEYS.foldedN, { count: 2, duration }),
    });
    expect(screen.queryByTestId('turn-trail-steps')).not.toBeInTheDocument();
    await userEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByTestId('turn-trail-steps')).toHaveTextContent(i18n.t(TURN_STEP_KEYS.choosePlanApproach));
  });

  it('says only how long a long turn took when it called no tool', () => {
    const trail = reduceTurnTrail(startTurnTrail(start), { type: 'finished', at: start + 30_000 });
    render(<FoldedTurnTrail trail={trail} />);
    expect(screen.getByTestId('turn-trail-folded')).toHaveTextContent(
      i18n.t(TURN_TRAIL_KEYS.foldedNoSteps, { duration: i18n.t(TURN_TRAIL_KEYS.elapsedSeconds, { seconds: 30 }) }),
    );
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });
});
