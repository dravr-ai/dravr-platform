// ABOUTME: Onboarding step (mobile) — the PAR-Q+ pre-participation screen
// ABOUTME: Mirrors the web OnboardingParq; a "yes" raises a coach-visible flag and never blocks sign-up

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, Pressable, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useQuery } from '@tanstack/react-query';
import { Button } from '../../components/ui';
import { useAuth } from '../../contexts/AuthContext';
import { userApi } from '../../services/api';
import { useOnboardingFlag } from '../../hooks/useOnboardingFlag';
import { useTranslation } from '@pierre/i18n';

/** Web-matching storage key prefix for this step. */
const STORAGE_PREFIX = 'dravr.parq_done.';

/**
 * The seven standard PAR-Q+ questions (mobile).
 *
 * The endpoints have existed and worked with no client calling them. A "yes"
 * never blocks sign-up — it writes a coach-visible medical flag with a 12-month
 * freshness horizon.
 */
export function OnboardingParqScreen() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const { mark } = useOnboardingFlag(STORAGE_PREFIX, user?.id);
  const [answers, setAnswers] = useState<Record<string, boolean>>({});
  const [saving, setSaving] = useState(false);

  const { data, isLoading, isError } = useQuery({
    queryKey: ['parq-questions'],
    queryFn: () => userApi.getParqQuestions(),
    staleTime: Infinity,
  });

  const questions = data?.questions ?? [];
  const allAnswered = questions.length > 0 && questions.every((q) => q.id in answers);

  const finish = async (status: 'complete' | 'skipped') => {
    if (saving) return;
    setSaving(true);
    if (status === 'complete') {
      try {
        await userApi.submitParq(questions.map((q) => ({ id: q.id, yes: answers[q.id] === true })));
      } catch {
        // Non-fatal: this screen is explicitly not a gate.
      }
    }
    userApi.setOnboardingStep('parq', status).catch(() => {});
    await mark();
  };

  // A screen the user can't answer is a screen they shouldn't be held on.
  React.useEffect(() => {
    if (isError) void finish('skipped');
    // eslint-disable-next-line react-hooks/exhaustive-deps -- fire once on failure
  }, [isError]);

  if (isLoading || isError) {
    return (
      <SafeAreaView className="flex-1 bg-surface items-center justify-center">
        <ActivityIndicator size="large" />
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView className="flex-1 bg-surface">
      <ScrollView contentContainerClassName="px-6 py-10">
        <Text className="text-2xl font-bold text-on-surface text-center">{t('onboarding.parqHeading')}</Text>
        <Text className="mt-3 text-sm text-on-surface-variant text-center">
          {t('onboarding.parqIntro')}
        </Text>

        <View className="mt-8 gap-3">
          {questions.map((q) => (
            <View
              key={q.id}
              className="rounded-lg border border-outline-variant bg-surface-container-low px-4 py-3"
            >
              <Text className="text-sm text-on-surface">{q.text}</Text>
              <View className="mt-3 flex-row gap-2">
                {[
                  { label: t('common.no'), value: false },
                  { label: t('common.yes'), value: true },
                ].map(({ label, value }) => {
                  const selected = answers[q.id] === value;
                  return (
                    <Pressable
                      key={label}
                      accessibilityRole="button"
                      accessibilityState={{ selected }}
                      onPress={() => setAnswers((prev) => ({ ...prev, [q.id]: value }))}
                      className={`rounded-md border px-4 py-1.5 ${
                        selected ? 'border-primary bg-primary' : 'border-outline-variant bg-surface'
                      }`}
                    >
                      <Text
                        className={selected ? 'text-on-primary text-xs' : 'text-on-surface-variant text-xs'}
                      >
                        {label}
                      </Text>
                    </Pressable>
                  );
                })}
              </View>
            </View>
          ))}
        </View>

        <View className="mt-8 gap-3">
          <Button
            title={saving ? t('app.saving') : allAnswered ? t('app.continue') : t('onboarding.parqAnswerAll')}
            onPress={() => void finish('complete')}
            disabled={saving || !allAnswered}
          />
          <Pressable onPress={() => void finish('skipped')} disabled={saving} accessibilityRole="button">
            <Text className="text-center text-sm text-on-surface-variant">{t('app.skipForNow')}</Text>
          </Pressable>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}
