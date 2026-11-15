// ABOUTME: Post-auth onboarding step — asks whether the user is an athlete, a coach, or both
// ABOUTME: Records the role (coaches others or not), never the reply style; a coach who does not train skips the athlete steps

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import type { OnboardingStatusResponse } from '@pierre/api-client';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { userApi } from '../services/api';
import OnboardingShell from './OnboardingShell';
import { Button } from './ui';
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
 *
 * A tap selects; the answer registers on Continue, so a mis-tap is corrected
 * by tapping another card rather than by a journey started in error.
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
  const [selected, setSelected] = useState<ProfileChoice | null>(null);
  const [choosing, setChoosing] = useState<ProfileChoice | null>(null);
  const [saveFailed, setSaveFailed] = useState(false);

  // The onboarding status carries `coaches_others`, which puts the coach
  // steps in the journey: it is read again before the step advances, so the
  // flow never decides the next step on the answer before this one.
  const setCoachingRole = useMutation({
    mutationFn: (coachesOthers: boolean) => userApi.setCoachingRole(coachesOthers),
    onSuccess: (_role, coachesOthers) => {
      // The role picks the journey, and the progress bar's segments with it:
      // the cached status carries it the moment the role is registered, and
      // the status is then read again before the step advances.
      queryClient.setQueryData<OnboardingStatusResponse>(
        QUERY_KEYS.user.onboardingStatus(),
        (cached) => cached && { ...cached, coaches_others: coachesOthers },
      );
      return Promise.all([
        queryClient.invalidateQueries({ queryKey: ['user'] }),
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() }),
      ]);
    },
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
      <div
        role="radiogroup"
        aria-label={t('onboarding.profileTypeIntro')}
        className="mt-8 grid gap-4 sm:grid-cols-3"
      >
        <ChoiceCard
          title={t('onboarding.imAnAthlete')}
          description={t('onboarding.athleteCardDescription')}
          checked={selected === 'athlete'}
          disabled={choosing !== null}
          onSelect={() => setSelected('athlete')}
        />
        <ChoiceCard
          title={t('humanCoach.iCoachOthers')}
          description={t('humanCoach.cardDescription')}
          checked={selected === 'coach'}
          disabled={choosing !== null}
          onSelect={() => setSelected('coach')}
        />
        <ChoiceCard
          title={t('humanCoach.iCoachAndTrain')}
          description={t('humanCoach.coachAndTrainCardDescription')}
          checked={selected === 'coach_and_athlete'}
          disabled={choosing !== null}
          onSelect={() => setSelected('coach_and_athlete')}
        />
      </div>
      {saveFailed ? (
        <p role="alert" className="mt-4 text-sm text-error text-center">
          {t('onboarding.profileTypeSaveFailed')}
        </p>
      ) : null}
      <div className="mx-auto mt-8 max-w-md">
        <Button
          variant="primary"
          onClick={() => {
            if (selected) void choose(selected);
          }}
          disabled={selected === null || choosing !== null}
          className="w-full"
          data-testid="profile-type-continue"
        >
          {choosing ? t('onboarding.settingUp') : t('onboarding.continueButton')}
        </Button>
      </div>
    </OnboardingShell>
  );
}

/** A single selectable profile option: one radio of the group, a full-height card. */
function ChoiceCard({
  title,
  description,
  checked,
  disabled,
  onSelect,
}: {
  title: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      onClick={onSelect}
      disabled={disabled}
      className={`flex flex-col items-start gap-2 rounded-xl border px-5 py-5 text-left transition-colors hover:border-primary disabled:cursor-not-allowed disabled:opacity-60 ${
        checked
          ? 'border-primary bg-primary-container/40'
          : 'ghost-border-strong bg-surface-container-lowest hover:bg-primary-container/40'
      }`}
    >
      <span className="font-display font-semibold text-base text-on-surface">{title}</span>
      <span className="text-sm text-on-surface-variant">{description}</span>
    </button>
  );
}
