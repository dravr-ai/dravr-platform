// ABOUTME: Onboarding step — the PAR-Q+ pre-participation screen, finally wired to a client
// ABOUTME: A "yes" raises a coach-visible flag and never blocks sign-up; the endpoints already existed

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { userApi } from '../services/api';
import { Button } from './ui';
import OnboardingShell from './OnboardingShell';
import { useTranslation } from '@pierre/i18n';

/**
 * The seven standard PAR-Q+ questions, served by the backend.
 *
 * The endpoints have existed and worked for months with no client calling them —
 * a finished safety feature behind a door nobody opened. This is the door.
 *
 * A "yes" never blocks sign-up. It writes a coach-visible medical flag with a
 * 12-month freshness horizon, so the coach heeds it and the athlete gets
 * re-screened rather than being interrogated on every conversation.
 */
export default function OnboardingParq({
  userDisplayName,
  onComplete,
}: {
  userDisplayName?: string | null;
  onComplete: (status?: 'complete' | 'skipped') => void;
}) {
  const { t } = useTranslation();
  const [answers, setAnswers] = useState<Record<string, boolean>>({});
  const [saving, setSaving] = useState(false);

  const { data, isLoading, isError } = useQuery({
    queryKey: ['parq-questions'],
    queryFn: () => userApi.getParqQuestions(),
    staleTime: Infinity,
  });

  const questions = data?.questions ?? [];
  const allAnswered = questions.length > 0 && questions.every((q) => q.id in answers);

  const handleSubmit = async () => {
    if (saving) return;
    setSaving(true);
    try {
      await userApi.submitParq(
        questions.map((q) => ({ id: q.id, yes: answers[q.id] === true })),
      );
    } catch {
      // Non-fatal: a failed write must not trap someone on a health screen that
      // is explicitly not a gate. The coach simply has one fewer flag.
    }
    onComplete('complete');
  };

  // A screen the user can't answer is a screen they shouldn't be held on.
  if (isError) {
    onComplete('skipped');
    return null;
  }

  if (isLoading) {
    return (
      <OnboardingShell heading={t('onboarding.parqHeading')}>
        <div className="flex flex-col items-center gap-4 py-8">
          <div className="pierre-spinner w-10 h-10 border-on-surface border-t-transparent" />
        </div>
      </OnboardingShell>
    );
  }

  return (
    <OnboardingShell
      heading={userDisplayName ? t('app.obParqGreeting', { name: userDisplayName }) : t('onboarding.parqHeading')}
    >
      <p className="mt-3 text-sm text-on-surface-variant text-center">
        {t('onboarding.parqIntro')}
      </p>

      <div className="mt-8 space-y-3">
        {questions.map((q) => (
          <div
            key={q.id}
            className="flex items-start justify-between gap-4 border-t ghost-border-faint py-3 first:border-t-0"
          >
            <span className="text-sm text-on-surface">{q.text}</span>
            <div className="flex shrink-0 gap-1" role="group" aria-label={q.text}>
              {[
                { label: t('common.no'), value: false },
                { label: t('common.yes'), value: true },
              ].map(({ label, value }) => {
                const selected = answers[q.id] === value;
                return (
                  <button
                    key={label}
                    type="button"
                    aria-pressed={selected}
                    onClick={() => setAnswers((prev) => ({ ...prev, [q.id]: value }))}
                    className={`rounded-md border px-3 py-1 text-xs font-medium transition-colors ${
                      selected
                        ? 'border-primary bg-primary text-on-primary'
                        : 'border-outline-variant bg-surface text-on-surface-variant hover:border-primary'
                    }`}
                  >
                    {label}
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </div>

      <div className="mt-8 space-y-3">
        <Button
          variant="primary"
          onClick={() => void handleSubmit()}
          disabled={saving || !allAnswered}
          className="w-full"
        >
          {saving ? t('onboarding.saving') : allAnswered ? t('onboarding.continueButton') : t('onboarding.parqAnswerAll')}
        </Button>
        <button
          type="button"
          onClick={() => onComplete('skipped')}
          disabled={saving}
          className="w-full text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
        >
          {t('onboarding.skipForNow')}
        </button>
      </div>
    </OnboardingShell>
  );
}
