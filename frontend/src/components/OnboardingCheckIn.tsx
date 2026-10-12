// ABOUTME: Onboarding step — how often the coach checks in on its own: every morning, three a week, weekly or never
// ABOUTME: A tap stores the cadence the check-in worker reads; never is an answer, and Settings changes it later (carnet#831)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import type { CheckInCadence } from '@pierre/shared-types';
import { CHECK_IN_OPTIONS } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import OnboardingShell from './OnboardingShell';
import { useCheckIn } from '../hooks/useCheckIn';

/**
 * The check-in cadence step.
 *
 * Asked with the other questions about the athlete, before the provider gate,
 * so the first morning after their data lands already follows their choice.
 * A choice is stored before the flow moves on: a failed write keeps the
 * athlete here to retry rather than advancing them past a question the server
 * never heard answered. Skipping stores nothing, which the worker reads as
 * never.
 */
export default function OnboardingCheckIn({
  onComplete,
}: {
  onComplete: (status: 'complete' | 'skipped') => void;
}) {
  const { t } = useTranslation();
  const checkIn = useCheckIn();
  const [choosing, setChoosing] = useState<CheckInCadence | null>(null);
  const [saveFailed, setSaveFailed] = useState(false);

  const choose = async (cadence: CheckInCadence) => {
    if (choosing) return;
    setChoosing(cadence);
    setSaveFailed(false);
    try {
      await checkIn.choose(cadence);
    } catch {
      setChoosing(null);
      setSaveFailed(true);
      return;
    }
    onComplete('complete');
  };

  return (
    <OnboardingShell heading={t('onboarding.checkInHeading')}>
      <p className="mt-3 text-sm text-on-surface-variant text-center">{t('onboarding.checkInIntro')}</p>
      <div className="mt-8 grid gap-3" role="group" aria-label={t('settings.selectCheckIn')}>
        {CHECK_IN_OPTIONS.map((option) => (
          <button
            key={option.value}
            type="button"
            data-testid={`check-in-option-${option.value}`}
            onClick={() => void choose(option.value)}
            disabled={choosing !== null}
            className="flex items-center justify-between gap-3 rounded-xl border ghost-border-strong bg-surface-container-lowest px-5 py-4 text-left transition-colors hover:border-primary hover:bg-primary-container/40 disabled:cursor-not-allowed disabled:opacity-60"
          >
            <span className="font-display font-semibold text-base text-on-surface">{t(option.labelKey)}</span>
            {choosing === option.value ? (
              <span className="pierre-spinner w-4 h-4 border-on-surface border-t-transparent" aria-hidden="true" />
            ) : null}
          </button>
        ))}
      </div>
      {saveFailed ? (
        <p role="alert" className="mt-4 text-sm text-error text-center">
          {t('settings.checkInSaveFailed')}
        </p>
      ) : null}
      <button
        type="button"
        onClick={() => onComplete('skipped')}
        disabled={choosing !== null}
        className="mt-6 w-full text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
      >
        {t('onboarding.skipForNow')}
      </button>
    </OnboardingShell>
  );
}
