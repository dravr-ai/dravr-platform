// ABOUTME: The small sheets Home's « Pour bien démarrer » rows open — the health check, the goal and the why, the check-in cadence
// ABOUTME: Each completes in place through the endpoint its onboarding screen used, ticks its row, and only then runs what was waiting

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS, withStepRecorded, type PersistedStepId } from '@pierre/shared-constants';
import type { OnboardingStatusResponse } from '@pierre/api-client';
import { userApi } from '../../services/api';
import { Sheet } from '../ui/Sheet';
import { ParqForm } from '../ParqForm';
import { useParqQuestions } from '../../hooks/useParqQuestions';
import { GoalWhyForm } from './GoalWhyForm';
import { CheckInForm } from './CheckInForm';

/** The rows that open a sheet, by what they ask. */
export type FirstStep = 'health_check' | 'goal_why' | 'check_in';

/** A sheet to open, and what runs once it is answered — never when it is only closed. */
export interface FirstStepRequest {
  step: FirstStep;
  /** « Construire mon plan » waits on the health check, then drafts its request. */
  onDone?: () => void;
}

/** The step row each sheet writes, the one its onboarding screen wrote. */
const STEP_ID: Record<FirstStep, PersistedStepId> = {
  health_check: 'parq',
  goal_why: 'about_you',
  check_in: 'check_in',
};

const TITLE_KEY: Record<FirstStep, string> = {
  health_check: 'home.gettingStarted.rows.healthCheck.title',
  goal_why: 'home.gettingStarted.rows.goalWhy.title',
  check_in: 'home.gettingStarted.rows.checkIn.title',
};

interface FirstStepSheetProps {
  request: FirstStepRequest;
  /** A bottom sheet on a phone, a side panel on a wider screen. */
  side: 'bottom' | 'right';
  onClose: () => void;
}

/**
 * One « Pour bien démarrer » sheet (carnet#1530). The answer goes through the
 * endpoint the onboarding screen used — the PAR-Q, the about-you answers, the
 * check-in cadence — then the step row is written `complete`, which is what
 * ticks the row on the server's list. Both writes are waited on: a refusal
 * keeps the sheet open with its error, so the count stays true. Closing the
 * sheet answers nothing.
 */
export function FirstStepSheet({ request, side, onClose }: FirstStepSheetProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const finish = async () => {
    const stepId = STEP_ID[request.step];
    await userApi.setOnboardingStep(stepId, 'complete');
    // The plan gate reads the cached status: it opens at once, not at the next read.
    queryClient.setQueryData<OnboardingStatusResponse>(
      QUERY_KEYS.user.onboardingStatus(),
      (cached) => cached && { ...cached, steps: withStepRecorded(cached.steps, stepId, 'complete') },
    );
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.home.gettingStarted() });
    onClose();
    request.onDone?.();
  };

  return (
    <Sheet side={side} title={t(TITLE_KEY[request.step])} onClose={onClose} data-testid={`first-step-sheet-${request.step}`}>
      <div className="px-4 py-4">
        {request.step === 'health_check' ? (
          <HealthCheckBody onAnswered={finish} />
        ) : request.step === 'goal_why' ? (
          <GoalWhyForm
            onSave={async (answers) => {
              await userApi.saveAboutYou(answers);
              await finish();
            }}
          />
        ) : (
          <CheckInForm onChosen={finish} />
        )}
      </div>
    </Sheet>
  );
}

/** The health check in the sheet: the onboarding form, without its skip — closing is putting it off. */
function HealthCheckBody({ onAnswered }: { onAnswered: () => Promise<void> }) {
  const { t } = useTranslation();
  const { data, isLoading, isError, refetch } = useParqQuestions();

  if (isLoading) {
    return (
      <div className="flex justify-center py-8" role="status" aria-label={t('common.loading')}>
        <div className="pierre-spinner w-6 h-6" />
      </div>
    );
  }
  if (isError || !data) {
    return (
      <div role="alert" className="flex flex-col items-start gap-3 py-4">
        <p className="text-sm text-on-surface">{t('onboarding.parq.loadFailed')}</p>
        <button type="button" onClick={() => void refetch()} className="rounded text-sm font-medium text-primary hover:underline focus-ring">
          {t('common.retry')}
        </button>
      </div>
    );
  }
  return (
    <>
      <p className="text-sm text-on-surface-variant">{t('onboarding.parq.intro')}</p>
      <div className="mt-4">
        <ParqForm
          questions={data.questions}
          stacked
          onSubmit={async (answers) => {
            await userApi.submitParq(answers);
            await onAnswered();
          }}
        />
      </div>
    </>
  );
}

export default FirstStepSheet;
