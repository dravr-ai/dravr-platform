// ABOUTME: Post-auth onboarding step — asks whether the user is an athlete, a coach, or both
// ABOUTME: Coach choices set coaching_persona=coach; a coach who does not train skips the athlete steps

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { userApi } from '../services/api';
import OnboardingShell from './OnboardingShell';
import { useTranslation } from '@pierre/i18n';

/** The three answers the step offers. */
type ProfileChoice = 'athlete' | 'coach' | 'coach_and_athlete';

/**
 * Athlete / coach / both onboarding step.
 *
 * Shown once, right after sign-in and before the connect-provider gate, so the
 * choice is in place before we infer a profile and propose agents. Both coach
 * answers persist `coaching_persona=coach`, which unlocks the coach-facing
 * builder personas (Taper Builder, etc.) in recommendations and the agent
 * library. Athletes keep the default Casual voice and never see those builder
 * tools. The choice is changeable later under Settings → Coaching style.
 *
 * What the answer does to the rest of the journey: a coach who does not train
 * is not asked about their own training or screened with the PAR-Q — those
 * steps leave the flow (`completeProfileType({ trains: false })`). Someone who
 * coaches and trains gets the coach tools and the athlete steps.
 */
export default function OnboardingProfileType({
  userDisplayName,
  onComplete,
}: {
  userDisplayName?: string | null;
  onComplete: (choice: { trains: boolean }) => void;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [choosing, setChoosing] = useState<ProfileChoice | null>(null);
  const [saveFailed, setSaveFailed] = useState(false);

  const setCoachPersona = useMutation({
    mutationFn: () => userApi.setCoachingPersona('coach'),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['user'] }),
  });

  const choose = async (choice: ProfileChoice) => {
    if (choosing) return;
    setChoosing(choice);
    setSaveFailed(false);
    if (choice === 'athlete') {
      // Nothing to write on the user row: `coaching_persona` has no "athlete"
      // variant — Casual IS the athlete default. What distinguishes "said
      // athlete" from "never answered" is the durable `profile_type` step row
      // `onComplete` writes.
      onComplete({ trains: true });
      return;
    }
    try {
      await setCoachPersona.mutateAsync();
    } catch {
      // The persona is what unlocks the coach tools this choice promises, so a
      // failed write keeps the user here to retry rather than advancing them
      // into a coach journey with an athlete's toolset.
      setChoosing(null);
      setSaveFailed(true);
      return;
    }
    onComplete({ trains: choice === 'coach_and_athlete' });
  };

  return (
    <OnboardingShell
      heading={
        userDisplayName
          ? t('onboarding.welcomeNamed', { name: userDisplayName })
          : t('onboarding.welcomeToDravr')
      }
    >
      <p className="mt-3 text-sm text-on-surface-variant text-center">
        {t('onboarding.profileTypeIntro')}
      </p>
      <div className="mt-8 grid gap-4 sm:grid-cols-3">
        <ChoiceCard
          title={t('onboarding.imAnAthlete')}
          description={t('onboarding.athleteCardDescription')}
          busy={choosing === 'athlete'}
          disabled={choosing !== null}
          onSelect={() => void choose('athlete')}
        />
        <ChoiceCard
          title={t('humanCoach.iCoachOthers')}
          description={t('humanCoach.cardDescription')}
          busy={choosing === 'coach'}
          disabled={choosing !== null}
          onSelect={() => void choose('coach')}
        />
        <ChoiceCard
          title={t('humanCoach.iCoachAndTrain')}
          description={t('humanCoach.coachAndTrainCardDescription')}
          busy={choosing === 'coach_and_athlete'}
          disabled={choosing !== null}
          onSelect={() => void choose('coach_and_athlete')}
        />
      </div>
      {saveFailed ? (
        <p role="alert" className="mt-4 text-sm text-error text-center">
          {t('onboarding.profileTypeSaveFailed')}
        </p>
      ) : null}
    </OnboardingShell>
  );
}

/** A single selectable profile option, rendered as a full-height button. */
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
  const { t } = useTranslation();
  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={disabled}
      className="flex flex-col items-start gap-2 rounded-xl border ghost-border-strong bg-surface-container-lowest px-5 py-5 text-left transition-colors hover:border-primary hover:bg-primary-container/40 disabled:cursor-not-allowed disabled:opacity-60"
    >
      <span className="font-display font-semibold text-base text-on-surface">{title}</span>
      <span className="text-sm text-on-surface-variant">{description}</span>
      {busy ? (
        <span className="mt-1 flex items-center gap-2 text-xs text-on-surface-variant">
          <span className="pierre-spinner w-4 h-4 border-on-surface border-t-transparent" />
          {t('onboarding.settingUp')}
        </span>
      ) : null}
    </button>
  );
}
