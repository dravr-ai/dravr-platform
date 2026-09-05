// ABOUTME: Onboarding progress indicator — labeled step dots (done / current / upcoming) for the onboarding journey
// ABOUTME: Fixed to the top of the viewport and pointer-events-none, so it never disturbs the centered step cards below

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { OnboardingProgressItem, OnboardingStepStatus } from '../onboarding/steps';
import { useTranslation } from '@pierre/i18n';

/**
 * The labeled step-dots progress indicator shown above every onboarding step.
 *
 * Rendered `fixed` at the top-center of the viewport and `pointer-events-none`
 * so it floats above the vertically-centered step cards without pushing them
 * off-screen or intercepting clicks. The step sequence is stable across steps
 * (the full canonical pipeline), so the indicator never reflows mid-flow.
 */
export default function OnboardingProgress({ steps }: { steps: OnboardingProgressItem[] }) {
  const { t } = useTranslation();
  const currentIndex = steps.findIndex((s) => s.status === 'current');
  const current = currentIndex >= 0 ? steps[currentIndex] : undefined;

  return (
    <nav
      aria-label={t('onboarding.progressAria')}
      className="pointer-events-none fixed inset-x-0 top-0 z-20 flex justify-center px-4 pt-6"
    >
      {current ? (
        <span className="sr-only">
          {t('onboarding.stepOfLabel', {
            current: currentIndex + 1,
            total: steps.length,
            label: t(current.labelKey),
          })}
        </span>
      ) : null}
      <ol className="flex items-start" aria-hidden="true">
        {steps.map((step, index) => (
          <li key={step.id} className="flex items-start">
            {index > 0 ? (
              <span
                className={`mt-[4px] h-0.5 w-8 rounded-full sm:w-12 ${
                  steps[index - 1].status === 'done' ? 'bg-primary' : 'bg-outline-variant'
                }`}
              />
            ) : null}
            <div className="flex min-w-16 flex-col items-center gap-1.5">
              <StepDot status={step.status} />
              <span
                className={`text-center text-xs leading-tight ${labelClass(step.status)}`}
              >
                {t(step.labelKey)}
              </span>
            </div>
          </li>
        ))}
      </ol>
    </nav>
  );
}

/** The dot marker for a single step, styled per its status. */
function StepDot({ status }: { status: OnboardingStepStatus }) {
  if (status === 'current') {
    return <span className="h-2.5 w-2.5 rounded-full bg-primary ring-2 ring-primary/25" />;
  }
  if (status === 'done') {
    return <span className="h-2.5 w-2.5 rounded-full bg-primary" />;
  }
  return <span className="h-2.5 w-2.5 rounded-full border border-outline-variant bg-surface" />;
}

function labelClass(status: OnboardingStepStatus): string {
  if (status === 'current') return 'font-medium text-on-surface';
  if (status === 'done') return 'text-on-surface-variant';
  return 'text-on-surface-variant/60';
}
