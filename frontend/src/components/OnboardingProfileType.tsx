// ABOUTME: Post-auth onboarding step — asks whether the user is an athlete or a coach
// ABOUTME: Choosing "coach" sets coaching_persona=coach (unlocks coach-facing builders); shown before provider connect

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { userApi } from '../services/api';
import OnboardingShell from './OnboardingShell';

/**
 * Athlete-vs-coach onboarding step.
 *
 * Shown once, right after sign-in and before the connect-provider gate, so the
 * choice is in place before we infer a profile and propose coaches. Picking
 * "I coach others" persists `coaching_persona=coach`, which unlocks the
 * coach-facing builder personas (Taper Builder, etc.) in recommendations and
 * the coach library. Athletes keep the default Casual voice and never see those
 * builder tools. The choice is changeable later under Settings → Coaching style.
 *
 * Completion is persisted per-user in `localStorage`, so this is a one-time
 * onboarding step rather than a recurring interstitial.
 */
export default function OnboardingProfileType({
  userDisplayName,
  onComplete,
}: {
  userDisplayName?: string | null;
  onComplete: () => void;
}) {
  const queryClient = useQueryClient();
  const [choosing, setChoosing] = useState<'athlete' | 'coach' | null>(null);

  const setCoachPersona = useMutation({
    mutationFn: () => userApi.setCoachingPersona('coach'),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['user'] }),
  });

  const handleAthlete = () => {
    if (choosing) return;
    setChoosing('athlete');
    // Nothing to write on the user row: `coaching_persona` has no "athlete"
    // variant — Casual IS the athlete default — so persisting one would mean
    // inventing an enum value to record a choice that is already recorded.
    //
    // What distinguishes "said athlete" from "never answered" is the durable
    // step record `onComplete` writes: a `profile_type` row with no
    // `coaching_persona=coach` alongside it means they chose athlete, and no row
    // at all means they were never asked. The screen's promise to tailor the
    // coaches is kept by the `about_you` step immediately after this one, which
    // writes the North Star, sport and goal that the coach proposal reads.
    onComplete();
  };

  const handleCoach = async () => {
    if (choosing) return;
    setChoosing('coach');
    try {
      await setCoachPersona.mutateAsync();
    } catch {
      // Non-fatal: proceed even if the persona write fails. A transient error
      // shouldn't trap the user on this screen — they can set the style later
      // in Settings; the default Casual start is harmless.
    }
    onComplete();
  };

  return (
    <OnboardingShell heading={userDisplayName ? `Welcome, ${userDisplayName}` : 'Welcome to Dravr'}>
      <p className="mt-3 text-sm text-on-surface-variant font-label text-center">
        How will you use Dravr? This tailors the coaches we suggest. You can change it anytime in
        Settings.
      </p>
      <div className="mt-8 grid gap-4 sm:grid-cols-2">
        <ChoiceCard
          title="I'm an athlete"
          description="Track your own training and get coaching tuned to how you actually train."
          busy={choosing === 'athlete'}
          disabled={choosing !== null}
          onSelect={handleAthlete}
        />
        <ChoiceCard
          title="I coach others"
          description="Build plans and manage the athletes you coach. Unlocks coach-facing plan builders."
          busy={choosing === 'coach'}
          disabled={choosing !== null}
          onSelect={() => void handleCoach()}
        />
      </div>
    </OnboardingShell>
  );
}

/** A single selectable athlete/coach option, rendered as a full-height button. */
function ChoiceCard({
  title,
  description,
  busy,
  disabled,
  onSelect,
}: {
  title: string;
  description: string;
  busy: boolean;
  disabled: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={disabled}
      className="flex flex-col items-start gap-2 rounded-xl border border-outline-variant bg-surface-container-low px-5 py-5 text-left transition-colors hover:border-primary hover:bg-surface-container disabled:cursor-not-allowed disabled:opacity-60"
    >
      <span className="font-display font-semibold text-base text-on-surface">{title}</span>
      <span className="text-sm text-on-surface-variant font-label">{description}</span>
      {busy ? (
        <span className="mt-1 flex items-center gap-2 text-xs text-on-surface-variant">
          <span className="pierre-spinner w-4 h-4 border-on-surface border-t-transparent" />
          Setting things up…
        </span>
      ) : null}
    </button>
  );
}
