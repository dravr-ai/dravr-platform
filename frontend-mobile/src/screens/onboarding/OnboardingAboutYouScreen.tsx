// ABOUTME: Onboarding step (mobile) — three questions about the athlete, before the provider gate
// ABOUTME: Mirrors the web OnboardingAboutYou; feeds the coach proposal, which already reads these

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, Pressable } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Button, Input } from '../../components/ui';
import { useAuth } from '../../contexts/AuthContext';
import { userApi } from '../../services/api';
import { useOnboardingFlag } from '../../hooks/useOnboardingFlag';
import { useTranslation } from '@pierre/i18n';
import { ONBOARDING_SPORTS as SPORTS, SPORT_LABEL_KEY } from '@pierre/shared-constants';

/** Sports offered as one-tap choices; anything else is typed. */


/** Web-matching storage key prefix for this step. */
const STORAGE_PREFIX = 'dravr.about_you_done.';

/**
 * Who the athlete is, in three questions (mobile).
 *
 * Runs before the provider gate for the same reason as on web: the coach
 * proposal reads a North Star and covered pillars and falls back to sport-mix
 * without them — and on a first-run connection there is no sport-mix either.
 * Every field is optional and the step is skippable.
 */
export function OnboardingAboutYouScreen() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const { mark } = useOnboardingFlag(STORAGE_PREFIX, user?.id);
  const [sport, setSport] = useState('');
  const [goal, setGoal] = useState('');
  const [northStar, setNorthStar] = useState('');
  const [saving, setSaving] = useState(false);

  const finish = async (status: 'complete' | 'skipped') => {
    if (saving) return;
    setSaving(true);
    if (status === 'complete') {
      try {
        await userApi.saveAboutYou({
          primary_sport: sport.trim() || undefined,
          goal: goal.trim() || undefined,
          north_star: northStar.trim() || undefined,
        });
      } catch {
        // Non-fatal: the answers are a head start, not a gate.
      }
    }
    userApi.setOnboardingStep('about_you', status).catch(() => {});
    await mark();
  };

  return (
    <SafeAreaView className="flex-1 bg-surface">
      <ScrollView contentContainerClassName="px-6 py-10" keyboardShouldPersistTaps="handled">
        <Text className="text-2xl font-bold text-on-surface text-center">
          {t('app.obTellMeTitle')}
        </Text>
        <Text className="mt-3 text-sm text-on-surface-variant text-center">
          {t('app.obThreeQuestions')}
        </Text>

        <Text className="mt-8 text-sm font-medium text-on-surface">{t('app.obSportQuestion')}</Text>
        <View className="mt-3 flex-row flex-wrap gap-2">
          {SPORTS.map((option) => {
            const selected = sport === option;
            return (
              <Pressable
                key={option}
                accessibilityRole="button"
                accessibilityState={{ selected }}
                onPress={() => setSport(selected ? '' : option)}
                className={`rounded-full border px-4 py-2 ${
                  selected ? 'border-primary bg-primary' : 'border-outline-variant bg-surface-container-low'
                }`}
              >
                <Text className={selected ? 'text-on-primary text-sm' : 'text-on-surface text-sm'}>
                  {t(SPORT_LABEL_KEY[option])}
                </Text>
              </Pressable>
            );
          })}
        </View>

        <Text className="mt-6 text-sm font-medium text-on-surface">
          {t('app.obGoalQuestion')}
        </Text>
        {/*
          The question above is the label, so `Input` renders without one: its
          own label is an uppercase eyebrow, which suits a field name but shouts
          a conversational prompt. Spacing is pinned to what the raw field used
          so the shared component changes the stroke, not the rhythm.
        */}
        <Input
          value={goal}
          onChangeText={setGoal}
          placeholder={t('app.obGoalPlaceholder')}
          containerStyle={{ marginTop: 8, marginBottom: 0 }}
        />

        <Text className="mt-6 text-sm font-medium text-on-surface">
          {t('app.obWhyMatters')}
        </Text>
        <Input
          value={northStar}
          onChangeText={setNorthStar}
          placeholder={t('app.obWhyPlaceholder')}
          multiline
          numberOfLines={3}
          containerStyle={{ marginTop: 8, marginBottom: 0 }}
        />
        <Text className="mt-1.5 text-xs text-on-surface-variant">
          {t('app.obWhyHint')}
        </Text>

        <View className="mt-8 gap-3">
          <Button title={saving ? t('app.saving') : t('app.continue')} onPress={() => void finish('complete')} disabled={saving} />
          <Pressable onPress={() => void finish('skipped')} disabled={saving} accessibilityRole="button">
            <Text className="text-center text-sm text-on-surface-variant">{t('app.skipForNow')}</Text>
          </Pressable>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}
