// ABOUTME: Home's « Ton point du jour » sheet body — how often the agent checks in on its own: every morning, three a week, weekly or never
// ABOUTME: A tap stores the cadence the check-in worker reads; never is an answer, and Settings changes it later (carnet#831)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import type { CheckInCadence } from '@pierre/shared-types';
import { CHECK_IN_OPTIONS } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { useCheckIn } from '../../hooks/useCheckIn';

/**
 * The check-in cadence — onboarding's check-in screen, moved to a « Pour bien
 * démarrer » row on Home (carnet#1530). A choice is stored, then handed to the
 * host, which ticks the row; a failed write keeps the athlete here to retry
 * rather than ticking a question the server never heard answered. Closing the
 * sheet stores nothing.
 */
export function CheckInForm({ onChosen }: { onChosen: () => Promise<void> }) {
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
      await onChosen();
    } catch {
      setChoosing(null);
      setSaveFailed(true);
    }
  };

  return (
    <div>
      <p className="text-sm text-on-surface-variant">{t('onboarding.checkInIntro')}</p>
      <div className="mt-6 grid gap-3" role="group" aria-label={t('settings.selectCheckIn')}>
        {CHECK_IN_OPTIONS.map((option) => (
          <button
            key={option.value}
            type="button"
            data-testid={`check-in-option-${option.value}`}
            onClick={() => void choose(option.value)}
            disabled={choosing !== null}
            className="flex min-h-11 items-center justify-between gap-3 rounded-xl border ghost-border-strong bg-surface-container-lowest px-5 py-4 text-left transition-colors hover:border-primary hover:bg-primary-container/40 focus-ring disabled:cursor-not-allowed disabled:opacity-60"
          >
            <span className="font-display font-semibold text-base text-on-surface">{t(option.labelKey)}</span>
            {choosing === option.value ? (
              <span className="pierre-spinner w-4 h-4 border-on-surface border-t-transparent" aria-hidden="true" />
            ) : null}
          </button>
        ))}
      </div>
      {saveFailed ? (
        <p role="alert" className="mt-4 text-sm text-error">
          {t('settings.checkInSaveFailed')}
        </p>
      ) : null}
    </div>
  );
}
