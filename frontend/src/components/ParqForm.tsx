// ABOUTME: The PAR-Q+ health check — seven questions shown verbatim, « Aucune de ces situations » in one tap, Non/Oui when some apply
// ABOUTME: Shared by onboarding's Santé screen and Home's « Bilan de santé » sheet; the host decides what an answer and a skip do

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { clsx } from 'clsx';
import type { ParqAnswer, ParqQuestion } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { Button } from './ui';

interface ParqFormProps {
  questions: ParqQuestion[];
  /**
   * Store the answers, one per question. A rejection keeps the form on screen
   * with an error and the same buttons to try again.
   */
  onSubmit: (answers: ParqAnswer[]) => Promise<void>;
  /** « Passer pour l'instant »; absent where putting it off is closing the sheet. */
  onSkip?: () => void;
  /** Each Non/Oui pair under its question at every width — a narrow sheet. */
  stacked?: boolean;
}

/**
 * The PAR-Q+ (carnet#1530): the seven standard questions, always visible and
 * word for word — the wording is the instrument (`intake.rs`). Most athletes
 * answer no to all seven, so « Aucune de ces situations » sends seven "no"
 * answers in one tap. « Certaines s'appliquent » reveals a Non/Oui pair per
 * question, 44px tall, and Continue waits until each has one. A "yes" never
 * blocks anything: it raises an agent-visible flag, which the note under the
 * questions says.
 */
export function ParqForm({ questions, onSubmit, onSkip, stacked = false }: ParqFormProps) {
  const { t } = useTranslation();
  const [answering, setAnswering] = useState(false);
  const [answers, setAnswers] = useState<Record<string, boolean>>({});
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);

  const allAnswered = questions.length > 0 && questions.every((q) => q.id in answers);
  const anyYes = Object.values(answers).some(Boolean);

  const submit = async (yes: (id: string) => boolean) => {
    if (saving) return;
    setSaving(true);
    setFailed(false);
    try {
      await onSubmit(questions.map((q) => ({ id: q.id, yes: yes(q.id) })));
    } catch {
      setFailed(true);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div>
      {answering ? (
        <p className="text-sm text-on-surface-variant" data-testid="parq-answer-each">
          {t('onboarding.parq.answerEach')}
        </p>
      ) : null}
      <ol className={clsx(answering ? 'mt-4' : 'mt-0')} aria-label={t('onboarding.stepHealthCheck')}>
        {questions.map((q, index) => (
          <li
            key={q.id}
            data-testid={`parq-question-${q.id}`}
            className={clsx(
              'flex flex-col gap-3 border-t ghost-border-faint py-3 first:border-t-0',
              !stacked && 'sm:flex-row sm:items-start sm:justify-between sm:gap-4',
            )}
          >
            <span className="flex min-w-0 gap-3 text-sm text-on-surface">
              <span aria-hidden="true" className="w-4 shrink-0 font-mono text-xs leading-5 text-outline">
                {index + 1}
              </span>
              <span>{q.text}</span>
            </span>
            {answering ? (
              <div className={clsx('flex shrink-0 gap-2 pl-7', !stacked && 'sm:pl-0')} role="group" aria-label={q.text}>
                {[
                  { label: t('common.no'), value: false, suffix: 'no' },
                  { label: t('common.yes'), value: true, suffix: 'yes' },
                ].map(({ label, value, suffix }) => {
                  const selected = answers[q.id] === value;
                  return (
                    <button
                      key={suffix}
                      type="button"
                      aria-pressed={selected}
                      disabled={saving}
                      data-testid={`parq-${q.id}-${suffix}`}
                      onClick={() => setAnswers((prev) => ({ ...prev, [q.id]: value }))}
                      className={clsx(
                        'h-11 min-w-16 rounded-lg border px-4 text-sm font-medium transition-colors focus-ring disabled:cursor-not-allowed',
                        selected
                          ? 'border-primary bg-primary text-on-primary'
                          : 'border-outline-variant bg-surface text-on-surface hover:border-primary',
                      )}
                    >
                      {label}
                    </button>
                  );
                })}
              </div>
            ) : null}
          </li>
        ))}
      </ol>

      {answering && anyYes ? (
        <p role="status" className="mt-4 rounded-lg bg-surface-container-low px-4 py-3 text-sm text-on-surface" data-testid="parq-yes-note">
          {t('onboarding.parq.yesNote')}
        </p>
      ) : null}

      {failed ? (
        <p role="alert" className="mt-4 text-sm text-error">
          {t('onboarding.parq.saveFailed')}
        </p>
      ) : null}

      <div className="mt-6 flex flex-col items-stretch gap-3">
        {answering ? (
          <>
            <Button
              variant="primary"
              onClick={() => void submit((id) => answers[id] === true)}
              disabled={saving || !allAnswered}
              className="w-full touch-target"
              data-testid="parq-continue"
            >
              {saving ? t('onboarding.saving') : allAnswered ? t('onboarding.continueButton') : t('onboarding.parqAnswerAll')}
            </Button>
            <button
              type="button"
              onClick={() => {
                setAnswering(false);
                setAnswers({});
              }}
              disabled={saving}
              className="touch-target text-sm font-medium text-on-surface-variant underline-offset-2 hover:text-on-surface hover:underline focus-ring rounded"
              data-testid="parq-back-to-none"
            >
              {t('onboarding.parq.backToNone')}
            </button>
          </>
        ) : (
          <>
            <Button
              variant="primary"
              onClick={() => void submit(() => false)}
              disabled={saving || questions.length === 0}
              className="w-full touch-target"
              data-testid="parq-none-apply"
            >
              {saving ? t('onboarding.saving') : t('onboarding.parq.noneApply')}
            </Button>
            <Button
              variant="secondary"
              onClick={() => setAnswering(true)}
              disabled={saving}
              className="w-full touch-target"
              data-testid="parq-some-apply"
            >
              {t('onboarding.parq.someApply')}
            </Button>
          </>
        )}
        {onSkip ? (
          <button
            type="button"
            onClick={onSkip}
            disabled={saving}
            className="touch-target text-sm font-medium text-on-surface-variant underline-offset-2 hover:text-on-surface hover:underline focus-ring rounded"
            data-testid="parq-skip"
          >
            {t('onboarding.skipForNow')}
          </button>
        ) : null}
      </div>
    </div>
  );
}
