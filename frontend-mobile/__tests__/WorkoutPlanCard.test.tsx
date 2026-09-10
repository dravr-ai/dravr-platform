// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile plan card renders the season and the fortnight the platform projected from the saved plan
// ABOUTME: Red if a field name reaches the screen, if no phase carries the now badge, or if a day's steps go missing

import React from 'react';
import { render } from '@testing-library/react-native';

import type { WorkoutPlan } from '@pierre/shared-types';
import WorkoutPlanCard from '../src/screens/chat/WorkoutPlanCard';

/**
 * The shape the platform sends, built like the server test's payload
 * (`crates/pierre-server/tests/plan_card_block_test.rs`) and identical to the
 * web card's fixture, so a divergence between the two cards shows up as a
 * failing assertion rather than as a screenshot nobody compares.
 */
function plan(overrides: Partial<WorkoutPlan> = {}): WorkoutPlan {
  return {
    goal_race: {
      name: 'Parkrun PB',
      date: '2026-11-14',
      discipline: 'run_5k',
      priority: 'A',
    },
    races: [
      { name: 'Club 10k', date: '2026-10-11', discipline: 'run_10k', priority: 'B' },
    ],
    season_start: '2026-09-14',
    season_end: '2026-11-16',
    flavour: {
      id: 'polarized-classic',
      label: 'mostly easy with two hard days',
      selected_by: 'coach',
    },
    phases: [
      {
        kind: 'build',
        start: '2026-09-14',
        end: '2026-10-26',
        weeks: 6,
        purpose: 'raise the ceiling',
        intent: 'two hard days, the rest easy',
        target_hours: 8,
        hard_sessions_max: 2,
        current: true,
      },
      {
        kind: 'taper',
        start: '2026-11-02',
        end: '2026-11-16',
        weeks: 2,
        purpose: 'arrive fresh',
        intent: 'sharpen',
        current: false,
      },
    ],
    current_phase_index: 0,
    weeks: [
      {
        week_start: '2026-09-14',
        focus: 'first build week',
        phase_index: 0,
        current: true,
        days: [
          {
            date: '2026-09-15',
            sport: 'run',
            workout: '4 x 8 min at threshold',
            duration_min: 65,
            intensity: 'threshold',
            rest: false,
            steps: [
              { label: 'Warm-up', duration_seconds: 900, target_zone: 'Z1' },
              { label: 'Threshold', duration_seconds: 480, target_zone: 'Threshold', repeat: 4 },
              { label: 'Cool-down', duration_seconds: 600, target_zone: 'Z1' },
            ],
            template_slug: 'threshold_4x8',
            template_source: 'catalogue',
            fueling: { carbs_g_per_h: 60, fluid_ml_per_h: 500, sodium_mg_per_h: 700 },
          },
          {
            date: '2026-09-17',
            sport: 'run',
            workout: 'off',
            intensity: '',
            rest: true,
          },
        ],
      },
      {
        week_start: '2026-09-21',
        focus: 'second build week',
        phase_index: 0,
        current: false,
        days: [
          {
            date: '2026-09-22',
            sport: 'run',
            workout: 'easy',
            duration_min: 60,
            intensity: 'Z2',
            rest: false,
          },
        ],
      },
    ],
    weeks_deferred: 4,
    ...overrides,
  };
}

describe('WorkoutPlanCard', () => {
  it('names the goal race and the approach the season runs on', () => {
    const { getByText } = render(<WorkoutPlanCard plan={plan()} />);

    expect(getByText('Training plan')).toBeTruthy();
    expect(getByText('Parkrun PB')).toBeTruthy();
    expect(getByText('mostly easy with two hard days')).toBeTruthy();
    expect(getByText(/chosen by your human coach/)).toBeTruthy();
  });

  it('lays every phase on the timeline and badges exactly the current one', () => {
    const { getByText, getAllByText } = render(<WorkoutPlanCard plan={plan()} />);

    expect(getByText('Build')).toBeTruthy();
    expect(getByText('Taper')).toBeTruthy();
    expect(getAllByText('now')).toHaveLength(1);
    expect(getByText(/two hard days, the rest easy/)).toBeTruthy();
    expect(getByText(/8 h\/wk/)).toBeTruthy();
    expect(getByText(/2 hard\/wk/)).toBeTruthy();
  });

  it('names this week and the next, and renders a day down to its steps', () => {
    const { getByText } = render(<WorkoutPlanCard plan={plan()} />);

    expect(getByText(/This week/)).toBeTruthy();
    expect(getByText(/Next week/)).toBeTruthy();
    expect(getByText('4 x 8 min at threshold')).toBeTruthy();
    expect(getByText(/65 min/)).toBeTruthy();
    expect(getByText(/Threshold · 8m · Threshold ×4/)).toBeTruthy();
    expect(getByText(/threshold_4x8/)).toBeTruthy();
    expect(getByText(/catalogue/)).toBeTruthy();
    expect(getByText(/60 g\/h carbs/)).toBeTruthy();
    expect(getByText('Rest')).toBeTruthy();
  });

  it('shows the other races the athlete named', () => {
    const { getByText } = render(<WorkoutPlanCard plan={plan()} />);
    expect(getByText(/Also racing/)).toBeTruthy();
    expect(getByText(/Club 10k/)).toBeTruthy();
  });

  it('says how many weeks the plan still holds beyond the card', () => {
    const { getByText } = render(<WorkoutPlanCard plan={plan()} />);
    expect(getByText(/4 more weeks in the plan/)).toBeTruthy();
  });

  it('never leaks a field name to the screen', () => {
    const { queryByText } = render(<WorkoutPlanCard plan={plan()} />);
    for (const field of ['goal_race', 'week_start', 'weeks_deferred', 'phase_index']) {
      expect(queryByText(new RegExp(field))).toBeNull();
    }
  });

  it('renders a plan that carries no flavour and no phases', () => {
    const { getByText, queryByText } = render(
      <WorkoutPlanCard
        plan={plan({ flavour: undefined, phases: [], current_phase_index: undefined, weeks_deferred: 0 })}
      />,
    );

    expect(getByText('Parkrun PB')).toBeTruthy();
    expect(queryByText('now')).toBeNull();
    expect(queryByText(/more weeks in the plan/)).toBeNull();
  });
});
