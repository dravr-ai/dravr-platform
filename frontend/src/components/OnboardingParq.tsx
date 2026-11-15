// ABOUTME: Onboarding's Santé — the PAR-Q+ as the third screen, after Tes données and before the team
// ABOUTME: One tap for the common answer; a "yes" raises an agent-visible flag and never blocks sign-up

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useRef } from 'react';
import { userApi } from '../services/api';
import OnboardingShell from './OnboardingShell';
import { ParqForm } from './ParqForm';
import { useParqQuestions } from '../hooks/useParqQuestions';
import { useTranslation } from '@pierre/i18n';

/**
 * The seven standard PAR-Q+ questions, served by the backend (carnet#1530).
 *
 * A "yes" never blocks sign-up. It writes an agent-visible medical flag with a
 * 12-month freshness horizon, so the agent heeds it and the athlete gets
 * re-screened rather than being interrogated on every conversation. Put off,
 * it becomes Home's first « Pour bien démarrer » row, and « Construire mon
 * plan » opens that sheet first.
 */
export default function OnboardingParq({
  userDisplayName,
  onComplete,
}: {
  userDisplayName?: string | null;
  onComplete: (status?: 'complete' | 'skipped') => void;
}) {
  const { t } = useTranslation();
  const { data, isLoading, isError } = useParqQuestions();

  // A screen the user can't answer is a screen they shouldn't be held on: a
  // failed read, or one that carries no question list, puts the step off.
  // Once, and after render — moving the flow on while rendering this screen
  // updates its parent mid-render, which React refuses.
  const unusable = isError || (data !== undefined && !Array.isArray(data.questions));
  const putOff = useRef(false);
  useEffect(() => {
    if (unusable && !putOff.current) {
      putOff.current = true;
      onComplete('skipped');
    }
  }, [unusable, onComplete]);
  if (unusable) return null;

  const heading = userDisplayName ? t('app.obParqGreeting', { name: userDisplayName }) : t('onboarding.parqHeading');

  if (isLoading || !data) {
    return (
      <OnboardingShell heading={heading}>
        <div className="flex flex-col items-center gap-4 py-8">
          <div className="pierre-spinner w-10 h-10 border-on-surface border-t-transparent" />
        </div>
      </OnboardingShell>
    );
  }

  return (
    <OnboardingShell heading={heading}>
      <p className="mt-3 text-sm text-on-surface-variant text-center">{t('onboarding.parq.intro')}</p>
      <div className="mx-auto mt-8 max-w-xl">
        <ParqForm
          questions={data.questions}
          onSubmit={async (answers) => {
            try {
              await userApi.submitParq(answers);
            } catch {
              // Non-fatal: a failed write must not trap someone on a health
              // screen that is explicitly not a gate. The agent simply has
              // one fewer flag.
            }
            onComplete('complete');
          }}
          onSkip={() => onComplete('skipped')}
        />
      </div>
    </OnboardingShell>
  );
}
