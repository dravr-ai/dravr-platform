// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The PAR-Q+ health check on a phone — seven questions verbatim, « Aucune de ces situations » in one tap, 44px Non/Oui when some apply
// ABOUTME: Shared by the onboarding Santé screen and Home's health-check sheet; mirrors the web ParqForm

import React, { useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { useQuery } from '@tanstack/react-query';
import type { ParqAnswer, ParqQuestion } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { userApi } from '../services/api';
import { Button } from './ui';

/** The seven questions, read once per session: the set never varies per athlete. */
export function useParqQuestions() {
  return useQuery({
    queryKey: ['parq-questions'],
    queryFn: () => userApi.getParqQuestions(),
    staleTime: Infinity,
  });
}

interface ParqFormProps {
  questions: ParqQuestion[];
  /** Store the answers; a rejection keeps the form with an error and the same buttons. */
  onSubmit: (answers: ParqAnswer[]) => Promise<void>;
  /** « Passer pour l'instant »; absent where putting it off is closing the sheet. */
  onSkip?: () => void;
}

/**
 * The PAR-Q+ (carnet#1530), as the phone board draws it: the seven standard
 * questions always visible and word for word — the wording is the
 * instrument. « Aucune de ces situations » sends seven "no" answers in one
 * tap; « Certaines s'appliquent » reveals a 44px Non/Oui pair per question,
 * and Continue waits until each has one. A "yes" never blocks anything, which
 * the note under the questions says.
 */
export function ParqForm({ questions, onSubmit, onSkip }: ParqFormProps) {
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
    <View>
      {answering ? (
        <Text className="text-sm text-text-secondary mb-3" testID="parq-answer-each">
          {t('onboarding.parq.answerEach')}
        </Text>
      ) : null}
      {questions.map((q, index) => (
        <View key={q.id} className="py-3 border-t border-border-faint" testID={`parq-question-${q.id}`}>
          <View className="flex-row">
            <Text className="w-5 text-xs font-mono text-text-tertiary leading-5">{index + 1}</Text>
            <Text className="flex-1 text-sm text-text-primary leading-5">{q.text}</Text>
          </View>
          {answering ? (
            <View className="mt-3 ml-5 flex-row gap-2" accessibilityRole="radiogroup" accessibilityLabel={q.text}>
              {[
                { label: t('common.no'), value: false, suffix: 'no' },
                { label: t('common.yes'), value: true, suffix: 'yes' },
              ].map(({ label, value, suffix }) => {
                const selected = answers[q.id] === value;
                return (
                  <Pressable
                    key={suffix}
                    onPress={() => setAnswers((prev) => ({ ...prev, [q.id]: value }))}
                    disabled={saving}
                    accessibilityRole="radio"
                    accessibilityState={{ selected }}
                    className={`min-h-11 min-w-16 items-center justify-center rounded-lg border px-5 ${
                      selected ? 'bg-primary border-primary' : 'border-border-strong'
                    }`}
                    testID={`parq-${q.id}-${suffix}`}
                  >
                    <Text className={`text-sm font-medium ${selected ? 'text-on-primary' : 'text-text-primary'}`}>
                      {label}
                    </Text>
                  </Pressable>
                );
              })}
            </View>
          ) : null}
        </View>
      ))}

      {answering && anyYes ? (
        <Text
          className="mt-4 rounded-lg bg-background-tertiary px-4 py-3 text-sm text-text-primary"
          accessibilityLiveRegion="polite"
          testID="parq-yes-note"
        >
          {t('onboarding.parq.yesNote')}
        </Text>
      ) : null}

      {failed ? (
        <Text className="mt-4 text-sm text-error" accessibilityLiveRegion="polite">
          {t('onboarding.parq.saveFailed')}
        </Text>
      ) : null}

      <View className="mt-6 gap-3">
        {answering ? (
          <>
            <Button
              title={saving ? t('app.saving') : allAnswered ? t('app.continue') : t('onboarding.parqAnswerAll')}
              onPress={() => void submit((id) => answers[id] === true)}
              disabled={saving || !allAnswered}
              testID="parq-continue"
            />
            <Pressable
              onPress={() => {
                setAnswering(false);
                setAnswers({});
              }}
              disabled={saving}
              accessibilityRole="button"
              className="min-h-11 items-center justify-center"
              testID="parq-back-to-none"
            >
              <Text className="text-sm text-text-secondary">{t('onboarding.parq.backToNone')}</Text>
            </Pressable>
          </>
        ) : (
          <>
            <Button
              title={saving ? t('app.saving') : t('onboarding.parq.noneApply')}
              onPress={() => void submit(() => false)}
              disabled={saving || questions.length === 0}
              testID="parq-none-apply"
            />
            <Button
              title={t('onboarding.parq.someApply')}
              variant="secondary"
              onPress={() => setAnswering(true)}
              disabled={saving}
              testID="parq-some-apply"
            />
          </>
        )}
        {onSkip ? (
          <Pressable
            onPress={onSkip}
            disabled={saving}
            accessibilityRole="button"
            className="min-h-11 items-center justify-center"
            testID="parq-skip"
          >
            <Text className="text-sm text-text-secondary">{t('app.skipForNow')}</Text>
          </Pressable>
        ) : null}
      </View>
    </View>
  );
}
