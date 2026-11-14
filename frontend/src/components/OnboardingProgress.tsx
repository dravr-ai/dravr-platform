// ABOUTME: Onboarding progress indicator — labeled step dots (done / skipped / current / upcoming) for the onboarding journey
// ABOUTME: Laid over the top of the onboarding step and pointer-events-none, so it never disturbs the centered step cards below

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { OnboardingProgressItem, OnboardingStepStatus } from '../onboarding/steps';
import { settledByInviteLine, settledByInviteSegments } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';

/**
 * The labeled step-dots progress indicator shown above every onboarding step.
 *
 * Rendered `absolute` at the top-center of the onboarding flow (OnboardingFlow
 * is its positioned parent) and `pointer-events-none`, so it floats above the
 * vertically-centered step cards without pushing them off-screen or
 * intercepting clicks. Absolute rather than fixed so it sits in the page's
 * stack: an App-level strip mounted above the flow (the offline strip) pushes
 * it down instead of being covered by it, and the step's reserved top room
 * moves down with it. It takes the notch inset as an out-of-flow layer
 * (`pad-safe-top`, index.css), so the step under it keeps its own inset too.
 *
 * The segments are the journey's, decided by the role (`onboardingProgress`),
 * so the count never moves mid-flow: a step put off keeps its place as a
 * dashed ring. The landmark's name says where the user is — "Step 2 of 4:
 * Your data" — and a caption under the dots says it visibly.
 *
 * A coach the team invited with their seat arrives with the role and the seat
 * already settled (`settledByInvite`, server truth): those segments stay on
 * the bar, done, and one line under it names them and says the invitation
 * settled them, so nothing on the bar is unexplained.
 */
export default function OnboardingProgress({
  steps,
  settledByInvite = false,
}: {
  steps: OnboardingProgressItem[];
  settledByInvite?: boolean;
}) {
  const { t, language } = useTranslation();
  const settledIds = new Set(settledByInviteSegments(steps, settledByInvite).map((s) => s.id));
  const settledLine = settledByInviteLine(t, steps, settledByInvite, language);
  const currentIndex = steps.findIndex((s) => s.status === 'current');
  const current = currentIndex >= 0 ? steps[currentIndex] : undefined;
  const position = { current: currentIndex + 1, total: steps.length };
  const whereAmI = current
    ? t('onboarding.stepOfLabel', { ...position, label: t(current.labelKey) })
    : undefined;

  return (
    <nav
      aria-label={whereAmI ?? t('onboarding.progressAria')}
      data-testid="onboarding-progress"
      className="pointer-events-none absolute inset-x-0 top-0 z-20 flex flex-col items-center gap-1.5 px-4 pt-6 pad-safe-top"
    >
      <ol className="flex items-start" aria-hidden="true">
        {steps.map((step, index) => (
          <li
            key={step.id}
            data-status={step.status}
            data-settled={settledIds.has(step.id) ? 'invite' : undefined}
            className="flex items-start"
          >
            {index > 0 ? (
              <span
                className={`mt-[4px] h-0.5 w-5 rounded-full lg:w-12 ${
                  isBehind(steps[index - 1].status) ? 'bg-primary' : 'bg-outline-variant'
                }`}
              />
            ) : null}
            <div className="flex flex-col items-center gap-1.5 lg:min-w-16">
              <StepDot status={step.status} />
              <span
                className={`hidden text-center text-xs leading-tight lg:block ${labelClass(step.status)}`}
              >
                {t(step.labelKey)}
              </span>
            </div>
          </li>
        ))}
      </ol>
      {current ? (
        <>
          {/* Under 1024px the labels do not fit, so the caption names this step too. */}
          <span
            aria-hidden="true"
            data-testid="onboarding-current-step"
            className="text-center text-xs leading-tight tabular-nums font-medium text-on-surface lg:hidden"
          >
            {whereAmI}
          </span>
          <span
            aria-hidden="true"
            data-testid="onboarding-step-counter"
            className="mt-0.5 hidden text-xs leading-tight tabular-nums text-on-surface-variant lg:block"
          >
            {t('onboarding.stepOf', position)}
          </span>
        </>
      ) : null}
      {settledLine ? (
        <span
          data-testid="onboarding-settled-by-invite"
          className="max-w-xs text-center text-xs leading-tight text-on-surface-variant"
        >
          {settledLine}
        </span>
      ) : null}
    </nav>
  );
}

/** Whether the user has moved past a step, finished or not. */
function isBehind(status: OnboardingStepStatus): boolean {
  return status === 'done' || status === 'skipped';
}

/** The dot marker for a single step, styled per its status. */
function StepDot({ status }: { status: OnboardingStepStatus }) {
  if (status === 'current') {
    return <span className="h-2.5 w-2.5 rounded-full bg-primary ring-2 ring-primary/25" />;
  }
  if (status === 'done') {
    return <span className="h-2.5 w-2.5 rounded-full bg-primary" />;
  }
  // Skipped keeps its place on the bar, as a broken ring; upcoming is a whole one.
  const ring = status === 'skipped' ? 'border-dashed' : 'border-solid';
  return (
    <span className={`h-2.5 w-2.5 rounded-full border-[1.5px] ${ring} border-outline bg-surface`} />
  );
}

/** Label ink: the current step in body ink, every other in full-strength secondary ink. */
function labelClass(status: OnboardingStepStatus): string {
  return status === 'current' ? 'font-medium text-on-surface' : 'text-on-surface-variant';
}
