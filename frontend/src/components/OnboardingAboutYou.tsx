// ABOUTME: Onboarding step — three questions about the athlete, before the provider gate
// ABOUTME: Feeds build_coach_proposal, which already reads these and falls back to sport-mix without them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { userApi } from '../services/api';
import { Button, Input, Textarea } from './ui';
import OnboardingShell from './OnboardingShell';

/** Sports offered as one-tap choices; anything else is typed. */
const SPORTS = ['Running', 'Cycling', 'Swimming', 'Triathlon', 'Strength', 'Hiking'] as const;

/**
 * Who the athlete is, in three questions.
 *
 * This runs *before* the provider gate on purpose. The coach proposal already
 * reads a North Star and covered pillars and explicitly falls back to sport-mix
 * when they're absent — and on a first-run connection there is no sport-mix
 * either, because the provider hasn't synced. Three answers we can always get
 * beat a data pull we don't control, and they're the only proposal input
 * guaranteed to exist when it runs.
 *
 * Every field is optional and the step is skippable: a partial answer is worth
 * strictly more than none, and a wall of required fields ahead of any value is
 * what people abandon.
 */
export default function OnboardingAboutYou({
  userDisplayName,
  onComplete,
}: {
  userDisplayName?: string | null;
  onComplete: (status?: 'complete' | 'skipped') => void;
}) {
  const [sport, setSport] = useState('');
  const [goal, setGoal] = useState('');
  const [northStar, setNorthStar] = useState('');
  const [saving, setSaving] = useState(false);

  const hasAnything = Boolean(sport.trim() || goal.trim() || northStar.trim());

  const handleContinue = async () => {
    if (saving) return;
    if (!hasAnything) {
      onComplete('skipped');
      return;
    }
    setSaving(true);
    try {
      await userApi.saveAboutYou({
        primary_sport: sport.trim() || undefined,
        goal: goal.trim() || undefined,
        north_star: northStar.trim() || undefined,
      });
    } catch {
      // Non-fatal, deliberately: the answers are a head start, not a gate. A
      // failed write must not trap someone on the step — the pillar walk asks
      // the same things conversationally later.
    }
    onComplete('complete');
  };

  return (
    <OnboardingShell
      heading={userDisplayName ? `Tell me about your training, ${userDisplayName}` : 'Tell me about your training'}
    >
      <p className="mt-3 text-sm text-on-surface-variant font-label text-center">
        Three quick questions so your coach starts out knowing you. Skip any of them — we&apos;ll
        pick the rest up as we go.
      </p>

      <div className="mt-8 space-y-6">
        <div>
          <span className="block text-sm font-medium text-on-surface">What do you mostly do?</span>
          <div className="mt-3 flex flex-wrap gap-2">
            {SPORTS.map((option) => {
              const selected = sport === option;
              return (
                <button
                  key={option}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => setSport(selected ? '' : option)}
                  className={`rounded-full border px-4 py-1.5 text-sm transition-colors ${
                    selected
                      ? 'border-primary bg-primary text-on-primary'
                      : 'border-outline-variant bg-surface-container-low text-on-surface hover:border-primary'
                  }`}
                >
                  {option}
                </button>
              );
            })}
          </div>
          <div className="mt-3">
            <Input
              id="about-sport-other"
              type="text"
              label="Or something else"
              placeholder="Type another sport"
              value={SPORTS.includes(sport as (typeof SPORTS)[number]) ? '' : sport}
              onChange={(e) => setSport(e.target.value)}
            />
          </div>
        </div>

        <div>
          <Input
            id="about-goal"
            type="text"
            label="What are you working toward?"
            placeholder="A first half-marathon in the spring, say"
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
          />
        </div>

        <div>
          <Textarea
            id="about-north-star"
            rows={3}
            label="And why does it matter to you?"
            placeholder="Keeping up with my kids on the trail"
            helpText="This is the one your coach comes back to when the training gets hard."
            value={northStar}
            onChange={(e) => setNorthStar(e.target.value)}
          />
        </div>
      </div>

      <div className="mt-8 space-y-3">
        <Button
          variant="primary"
          onClick={() => void handleContinue()}
          disabled={saving}
          className="w-full"
        >
          {saving ? 'Saving…' : 'Continue'}
        </Button>
        <button
          type="button"
          onClick={() => onComplete('skipped')}
          disabled={saving}
          className="w-full text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
        >
          Skip for now
        </button>
      </div>
    </OnboardingShell>
  );
}
