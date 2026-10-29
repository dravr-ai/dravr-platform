// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile plan card renders the season and the fortnight the platform projected from the saved plan
// ABOUTME: Red if a field name reaches the screen, if no phase carries the now badge, or if a day's steps go missing

import React from 'react';
import { act, render, within } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';

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

  it('lays every phase on the timeline and marks exactly the current one', () => {
    const { getByText, getAllByText } = render(<WorkoutPlanCard plan={plan()} />);

    // The current phase and its `now` word take the primary ink; the other
    // phase stays secondary. Neither is a pill.
    expect(getByText('Build').props.className).toContain('text-primary');
    expect(getByText('Taper').props.className).toContain('text-text-secondary');
    expect(getByText('Taper').props.className).not.toContain('rounded');
    expect(getAllByText('now')).toHaveLength(1);
    expect(getByText('now').props.className).toContain('text-primary');
    expect(getByText('now').props.className).not.toContain('rounded-full');
    expect(getByText(/two hard days, the rest easy/)).toBeTruthy();
    // The weekly figures are their own mono spans.
    expect(getByText('8 h/wk').props.className).toContain('font-mono');
    expect(getByText('2 hard/wk').props.className).toContain('font-mono');
    expect(getByText('6 wk').props.className).toContain('font-mono');
  });

  it('names this week and the next, and renders a day down to its steps', () => {
    const { getByText, getByTestId } = render(<WorkoutPlanCard plan={plan()} />);

    expect(getByText(/This week/)).toBeTruthy();
    expect(getByText(/Next week/)).toBeTruthy();
    expect(within(getByTestId('workout-plan-week-1')).getByText('September 14').props.className).toContain(
      'font-mono',
    );

    const day = within(getByTestId('workout-plan-day-2026-09-15'));
    expect(day.getByText('Tue, Sep 15').props.className).toContain('font-mono');
    expect(day.getByText('4 x 8 min at threshold')).toBeTruthy();
    // The duration is a mono span of its own; the sentence around it still
    // reads "· Run · 65 min · threshold".
    expect(day.getByText('65 min').props.className).toContain('font-mono');
    expect(day.getByText(/· Run ·.*· threshold$/)).toBeTruthy();
    // A step is one sentence whose duration and repeat count are mono spans.
    expect(day.getByText(/Threshold · 8m · Threshold ×4/)).toBeTruthy();
    expect(day.getByText('8m').props.className).toContain('font-mono');
    expect(day.getByText('×4').props.className).toContain('font-mono');
    expect(day.getByText(/threshold_4x8/)).toBeTruthy();
    expect(day.getByText(/catalogue/)).toBeTruthy();
    expect(day.getByText(/60 g\/h carbs/)).toBeTruthy();
    expect(within(getByTestId('workout-plan-day-2026-09-17')).getByText('Rest')).toBeTruthy();
  });

  it("tells a flagged athlete why a day's fuelling amounts are withheld, and shows no rate", () => {
    const flagged = plan();
    flagged.weeks[0].days[0] = {
      date: '2026-09-15',
      sport: 'run',
      workout: '4 x 8 min at threshold',
      duration_min: 65,
      intensity: 'threshold',
      rest: false,
      fueling_withheld: {
        reason: 'medical_flag',
        set_by: 'clinician',
        note: 'A medical/PAR-Q flag is on file for this athlete.',
      },
    };
    const { getByTestId } = render(<WorkoutPlanCard plan={flagged} />);

    const day = within(getByTestId('workout-plan-day-2026-09-15'));
    expect(
      day.getByText(/amounts withheld: a medical flag is on file, so your clinician sets them/),
    ).toBeTruthy();
    expect(day.queryByText(/g\/h/)).toBeNull();
  });

  it('names the fuel products a day uses, even when its amounts are withheld', () => {
    const flagged = plan();
    flagged.weeks[0].days[0] = {
      date: '2026-09-15',
      sport: 'run',
      workout: 'long run',
      duration_min: 120,
      intensity: 'Z2',
      rest: false,
      fueling_withheld: {
        reason: 'medical_flag',
        set_by: 'clinician',
        note: 'A medical/PAR-Q flag is on file for this athlete.',
      },
      fuel_products: [
        { product_id: '7b0c0d3e-2f4b-4b9e-9b7e-1d2c3a4b5c6d', name: 'Maurten Gel 100 (Caf 100)' },
        { product_id: '0d7f7a44-5d2c-4e57-8c5a-6b1a2e3f4d5c', name: 'Näak Drink Mix' },
      ],
    };
    const { getByTestId } = render(<WorkoutPlanCard plan={flagged} />);

    const day = within(getByTestId('workout-plan-day-2026-09-15'));
    expect(day.getByText(/Maurten Gel 100 \(Caf 100\) · Näak Drink Mix/)).toBeTruthy();
    expect(day.queryByText(/g\/h/)).toBeNull();
  });

  it('shows the other races the athlete named', () => {
    const { getByText } = render(<WorkoutPlanCard plan={plan()} />);
    expect(getByText(/Also racing/)).toBeTruthy();
    expect(getByText(/Club 10k/)).toBeTruthy();
    expect(getByText('Oct 11, 2026').props.className).toContain('font-mono');
  });

  it('says how many weeks the plan still holds beyond the card', () => {
    const { getByText } = render(<WorkoutPlanCard plan={plan()} />);
    expect(getByText(/4 more weeks in the plan/)).toBeTruthy();
    expect(getByText('4').props.className).toContain('font-mono');
  });

  // carnet#1212: both counts had one plural form, « 1 more weeks » and
  // « 1 dures/sem »; countKey now picks the singular, and the figure stays mono.
  it('reads one deferred week and one hard session in the singular', async () => {
    const one = plan({ weeks_deferred: 1 });
    const [build, taper] = one.phases ?? [];
    one.phases = [{ ...build, hard_sessions_max: 1 }, taper];
    const english = render(<WorkoutPlanCard plan={one} />);
    expect(english.getByText(/^1 more week in the plan$/)).toBeTruthy();
    english.unmount();
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    try {
      const { getByText } = render(<WorkoutPlanCard plan={one} />);
      expect(getByText(/^1 semaine de plus dans le plan$/)).toBeTruthy();
      expect(getByText('1 dure/sem').props.className).toContain('font-mono');
    } finally {
      await act(async () => {
        await i18n.changeLanguage('en');
      });
    }
  });

  // Boreal v2.2 P3: the card is one faint outline on the paper — no header
  // fill, no interior hairline but the one under the header block.
  it('is one outline with a single hairline under its header', () => {
    const { getByTestId, toJSON } = render(<WorkoutPlanCard plan={plan()} />);

    const card = getByTestId('workout-plan-card');
    expect(card.props.className).toContain('border-border-faint');
    expect(card.props.style).toBeUndefined();

    const json = JSON.stringify(toJSON());
    expect(json).not.toContain('surfaceContainerHigh');
    expect(json).not.toContain('borderTopWidth');
    expect(json).not.toContain('borderBottomWidth');
    const hairlines = json.match(/border-b border-border-faint/g) ?? [];
    expect(hairlines).toHaveLength(1);
  });

  // The card printed « 2026-09-15 » and « run » under French chrome
  // (carnet#1191): every date is a date in the athlete's language, and a
  // sport is its catalogue word.
  it("reads dates and sports in the athlete's language, never the wire spelling", async () => {
    await i18n.changeLanguage('fr');
    try {
      const { getByText, getByTestId, queryByText } = render(<WorkoutPlanCard plan={plan()} />);

      expect(getByText('14 nov. 2026')).toBeTruthy();
      expect(getByText('14 sept. 2026')).toBeTruthy();
      expect(getByText('16 nov. 2026')).toBeTruthy();
      const day = within(getByTestId('workout-plan-day-2026-09-15'));
      expect(day.getByText('mar. 15 sept.')).toBeTruthy();
      expect(day.getByText(/· Course ·/)).toBeTruthy();
      expect(day.queryByText(/· run ·/)).toBeNull();
      expect(within(getByTestId('workout-plan-day-2026-09-17')).getByText('Repos')).toBeTruthy();
      expect(within(getByTestId('workout-plan-week-1')).getByText('14 septembre')).toBeTruthy();
      expect(queryByText(/\d{4}-\d{2}-\d{2}/)).toBeNull();
    } finally {
      await i18n.changeLanguage('en');
    }
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
