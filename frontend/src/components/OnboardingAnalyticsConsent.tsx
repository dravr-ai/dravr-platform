// ABOUTME: Onboarding step — the usage-analytics question for an account no form asked: Google sign-in, chat sign-up, older accounts
// ABOUTME: Yes and No weigh the same and either is stored before the flow moves on; Settings changes it later (carnet#1366)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { ANALYTICS_POLICY_URL } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import OnboardingShell from './OnboardingShell';

/**
 * The analytics consent step.
 *
 * Shown once to an account that has never answered: the sign-up form asks
 * with its own box, so this meets the accounts that came in without one. Law
 * 25 wants the consent manifest, free and specific, so it is its own question
 * with nothing bundled, and declining takes the same one tap as accepting.
 * The answer is stored before the flow moves on: a failed write keeps the
 * person here to retry instead of asking again at every sign-in.
 */
export default function OnboardingAnalyticsConsent({
  onAnswer,
}: {
  onAnswer: (enabled: boolean) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [answering, setAnswering] = useState<boolean | null>(null);
  const [saveFailed, setSaveFailed] = useState(false);

  const answer = async (enabled: boolean) => {
    if (answering !== null) return;
    setAnswering(enabled);
    setSaveFailed(false);
    try {
      await onAnswer(enabled);
    } catch {
      setAnswering(null);
      setSaveFailed(true);
    }
  };

  const choices = [
    { enabled: true, labelKey: 'onboarding.analyticsConsentAccept', testId: 'analytics-consent-accept' },
    { enabled: false, labelKey: 'onboarding.analyticsConsentDecline', testId: 'analytics-consent-decline' },
  ];

  return (
    <OnboardingShell heading={t('onboarding.analyticsConsentTitle')}>
      <p className="mt-3 text-sm text-on-surface-variant text-center">
        {t('onboarding.analyticsConsentBody')}
      </p>
      <p className="mt-2 text-sm text-center">
        <a
          href={ANALYTICS_POLICY_URL}
          target="_blank"
          rel="noopener noreferrer"
          className="text-primary hover:text-primary-fixed-dim font-medium underline-offset-2 hover:underline"
        >
          {t('auth.analyticsConsentPolicyLink')}
        </a>
      </p>
      <div className="mt-8 grid gap-3" role="group" aria-label={t('onboarding.analyticsConsentTitle')}>
        {choices.map((choice) => (
          <button
            key={choice.testId}
            type="button"
            data-testid={choice.testId}
            onClick={() => void answer(choice.enabled)}
            disabled={answering !== null}
            className="flex items-center justify-between gap-3 rounded-xl border ghost-border-strong bg-surface-container-lowest px-5 py-4 text-left transition-colors hover:border-primary hover:bg-primary-container/40 disabled:cursor-not-allowed disabled:opacity-60"
          >
            <span className="font-display font-semibold text-base text-on-surface">{t(choice.labelKey)}</span>
            {answering === choice.enabled ? (
              <span className="pierre-spinner w-4 h-4 border-on-surface border-t-transparent" aria-hidden="true" />
            ) : null}
          </button>
        ))}
      </div>
      {saveFailed ? (
        <p role="alert" className="mt-4 text-sm text-error text-center">
          {t('onboarding.analyticsConsentSaveFailed')}
        </p>
      ) : null}
    </OnboardingShell>
  );
}
