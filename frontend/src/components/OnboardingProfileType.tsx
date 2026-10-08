// ABOUTME: Post-auth onboarding step — asks whether the user is an athlete, a coach, or both
// ABOUTME: Records the role (coaches others or not), never the reply style; a coach who does not train skips the athlete steps

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
 * choice is in place before we infer a profile and propose agents. The answer
 * records a role — whether the user coaches others — which puts the group step
 * in their journey and the coach-facing builder personas (Taper Builder, etc.)
 * in their recommendations and agent library. It never sets the reply style:
 * that stays the user's own (Casual by default) and is chosen under Settings →
 * Coaching style, so a coach talking about their own training keeps their own
 * voice (carnet#827). The athlete answer records the role as off, so a former
 * coach who re-onboards as an athlete loses the coach tools.
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

  const setCoachingRole = useMutation({
    mutationFn: (coachesOthers: boolean) => userApi.setCoachingRole(coachesOthers),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['user'] }),
  });

  const choose = async (choice: ProfileChoice) => {
    if (choosing) return;
    setChoosing(choice);
    setSaveFailed(false);
    try {
      await setCoachingRole.mutateAsync(choice !== 'athlete');
    } catch {
      // The role decides which journey and toolset follow, so a failed write
      // keeps the user here to retry rather than advancing them into a
      // journey the server does not know they chose.
      setChoosing(null);
      setSaveFailed(true);
      return;
    }
    onComplete({ trains: choice !== 'coach' });
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
