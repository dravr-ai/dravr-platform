// ABOUTME: The plan card renders the season and the fortnight the platform projected from the saved plan
// ABOUTME: Red if a field name leaks to the page, if no phase carries the now badge, or if a day's steps go missing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import { i18n } from '@pierre/i18n';
import type { WorkoutPlan } from '@pierre/shared-types';
import WorkoutPlanCard from '../WorkoutPlanCard';

/**
 * The shape the platform sends, built like the server test's payload
 * (`crates/pierre-server/tests/plan_card_block_test.rs`): a build phase
 * covering today, a taper after it, and one week whose Tuesday runs a
 * catalogue template.
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
    render(<WorkoutPlanCard plan={plan()} />);

    expect(screen.getByText('Training plan')).toBeInTheDocument();
    expect(screen.getByText('Parkrun PB')).toBeInTheDocument();
    expect(screen.getByText('Nov 14, 2026')).toBeInTheDocument();
    expect(screen.getByText('mostly easy with two hard days')).toBeInTheDocument();
    expect(screen.getByText(/chosen by your human coach/)).toBeInTheDocument();
  });

  it('lays every phase on the timeline and badges exactly the current one', () => {
    render(<WorkoutPlanCard plan={plan()} />);

    expect(screen.getByText('Build')).toBeInTheDocument();
    expect(screen.getByText('Taper')).toBeInTheDocument();
    // One badge, on the phase covering today.
    expect(screen.getAllByText('now')).toHaveLength(1);
    // The current phase's rhythm, from the kernel's own numbers.
    expect(screen.getByText(/two hard days, the rest easy/)).toBeInTheDocument();
    expect(screen.getByText(/8 h\/wk/)).toBeInTheDocument();
    expect(screen.getByText(/2 hard\/wk/)).toBeInTheDocument();
  });

  it('names this week and the next, and renders a day down to its steps', () => {
    render(<WorkoutPlanCard plan={plan()} />);

    expect(screen.getByText('This week')).toBeInTheDocument();
    expect(screen.getByText('Next week')).toBeInTheDocument();
    expect(screen.getByText('first build week')).toBeInTheDocument();

    expect(screen.getByText('4 x 8 min at threshold')).toBeInTheDocument();
    expect(screen.getByText(/65 min/)).toBeInTheDocument();
    // The repeated step carries its count.
    expect(screen.getByText('Threshold · 8m · Threshold ×4')).toBeInTheDocument();
    expect(screen.getByText('Warm-up · 15m · Z1')).toBeInTheDocument();
    // The template and the tier it resolved in.
    expect(screen.getByText('threshold_4x8')).toBeInTheDocument();
    expect(screen.getByText(/catalogue/)).toBeInTheDocument();
    // Fuelling reaches the athlete.
    expect(screen.getByText(/60 g\/h carbs/)).toBeInTheDocument();
    // A rest day is the one word.
    expect(screen.getByText('Rest')).toBeInTheDocument();
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
    render(<WorkoutPlanCard plan={flagged} />);

    expect(
      screen.getByText(/amounts withheld: a medical flag is on file, so your clinician sets them/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/g\/h/)).not.toBeInTheDocument();
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
    render(<WorkoutPlanCard plan={flagged} />);

    expect(screen.getByTestId('plan-day-fuel')).toHaveTextContent(
      'Fuel amounts withheld: a medical flag is on file, so your clinician sets them · Maurten Gel 100 (Caf 100), Näak Drink Mix',
    );
    expect(screen.queryByText(/g\/h/)).not.toBeInTheDocument();
  });

  it("puts a day's rates and its fuel products on one Fuel row", () => {
    const fuelled = plan();
    fuelled.weeks[0].days[0] = {
      date: '2026-09-15',
      sport: 'run',
      workout: 'long run',
      duration_min: 120,
      intensity: 'Z2',
      rest: false,
      fueling: { carbs_g_per_h: 60, fluid_ml_per_h: 600 },
      fuel_products: [
        { product_id: '7b0c0d3e-2f4b-4b9e-9b7e-1d2c3a4b5c6d', name: 'Maurten Gel 100' },
        { product_id: '0d7f7a44-5d2c-4e57-8c5a-6b1a2e3f4d5c', name: 'Näak Drink Mix' },
      ],
    };
    render(<WorkoutPlanCard plan={fuelled} />);

    // One row, one Fuel label: the rates, then the products.
    expect(screen.getByTestId('plan-day-fuel')).toHaveTextContent(
      'Fuel 60 g/h carbs · 600 ml/h fluid · Maurten Gel 100, Näak Drink Mix',
    );
    expect(screen.getAllByText('Fuel')).toHaveLength(1);
  });

  it('shows the other races the athlete named', () => {
    render(<WorkoutPlanCard plan={plan()} />);
    expect(screen.getByText(/Also racing/)).toBeInTheDocument();
    expect(screen.getByText(/Club 10k/)).toBeInTheDocument();
  });

  it('says how many weeks the plan still holds beyond the card', () => {
    render(<WorkoutPlanCard plan={plan()} />);
    expect(screen.getByText(/4 more weeks in the plan/)).toBeInTheDocument();
  });

  // carnet#1212: both counts had one plural form, so « 1 more weeks » and
  // « 1 dures/sem »; countKey now picks the singular.
  it('reads one deferred week and one hard session in the singular', async () => {
    const one = plan({ weeks_deferred: 1 });
    const [build, taper] = one.phases ?? [];
    one.phases = [{ ...build, hard_sessions_max: 1 }, taper];
    const { unmount } = render(<WorkoutPlanCard plan={one} />);
    expect(screen.getByText('1 more week in the plan')).toBeInTheDocument();
    unmount();
    await i18n.changeLanguage('fr');
    try {
      render(<WorkoutPlanCard plan={one} />);
      expect(screen.getByText('1 semaine de plus dans le plan')).toBeInTheDocument();
      expect(screen.getByText(/· 1 dure\/sem/)).toBeInTheDocument();
    } finally {
      await i18n.changeLanguage('en');
    }
  });

  // The card printed « 2026-09-15 » and « run » / « rest » under French
  // chrome (carnet#1191): every date is a date in the athlete's language, and
  // a sport is its catalogue word.
  it('reads dates and sports in the athlete\'s language, never the wire spelling', async () => {
    await i18n.changeLanguage('fr');
    try {
      const { container } = render(<WorkoutPlanCard plan={plan()} />);

      expect(screen.getByText('14 nov. 2026')).toBeInTheDocument();
      expect(screen.getByText(/14 sept\. 2026 → 16 nov\. 2026/)).toBeInTheDocument();
      const session = screen.getByText('4 x 8 min at threshold').closest('tr');
      expect(session).not.toBeNull();
      expect(within(session as HTMLElement).getByText('mar. 15 sept.')).toBeInTheDocument();
      expect(within(session as HTMLElement).getByText('Course')).toBeInTheDocument();
      // A rest day names no sport: its one word is the day.
      const rest = screen.getByText('Repos').closest('tr');
      expect(rest).not.toBeNull();
      expect(rest?.textContent).toBe('jeu. 17 sept.Repos');
      // The week that is not named reads its Monday as a date.
      expect(screen.getByText('14 septembre')).toBeInTheDocument();

      expect(container.textContent).not.toMatch(/\d{4}-\d{2}-\d{2}/);
      for (const raw of ['run', 'rest']) {
        expect(screen.queryByText(raw)).toBeNull();
      }
    } finally {
      await i18n.changeLanguage('en');
    }
  });

  it('never leaks a field name to the page', () => {
    const { container } = render(<WorkoutPlanCard plan={plan()} />);
    for (const field of ['goal_race', 'week_start', 'template_slug', 'weeks_deferred', 'phase_index']) {
      expect(container.textContent).not.toContain(field);
    }
  });

  it('renders a plan that carries no flavour and no phases', () => {
    render(
      <WorkoutPlanCard
        plan={plan({ flavour: undefined, phases: [], current_phase_index: undefined, weeks_deferred: 0 })}
      />,
    );

    expect(screen.getByText('Parkrun PB')).toBeInTheDocument();
    expect(screen.queryByText('now')).not.toBeInTheDocument();
    expect(screen.queryByText(/more weeks in the plan/)).not.toBeInTheDocument();
  });
});
