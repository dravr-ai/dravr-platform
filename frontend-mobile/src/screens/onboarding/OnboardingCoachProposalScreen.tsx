// ABOUTME: Post-connect onboarding step (mobile) — "analyzing your data" then inferred profile + top-3 coaches
// ABOUTME: Mirrors the web OnboardingCoachProposal; backed by GET /api/coaches/proposal

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useQuery } from '@tanstack/react-query';
import type { ProposedCoach } from '@pierre/shared-types';
import { Card, Button } from '../../components/ui';
import { coachesApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useCoachProposalSeen } from '../../hooks/useCoachProposalSeen';
import { useTranslation } from '@pierre/i18n';
import { activitySportLabelKey, coachCategoryLabelKey } from '@pierre/shared-constants';
import { useRouter } from 'expo-router';
import { useConversations } from '../chat/useConversations';
import { threadHref } from '../../navigation/routes';

/**
 * Onboarding coach proposal (mobile).
 *
 * Reached only via RootLayoutNav when the user has a connected provider but
 * hasn't completed this step. On mount it calls `GET /api/coaches/proposal`
 * (activity scan + LLM re-rank) and shows an "analyzing your data" spinner
 * while it runs, then the inferred sport profile and up-to-3 coaches with a
 * rationale each. Completing the step flips the shared `useCoachProposalSeen`
 * cache, which routes the user on to chat.
 */
export function OnboardingCoachProposalScreen() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const { markSeen } = useCoachProposalSeen(user?.id);
  const [selecting, setSelecting] = useState<string | null>(null);
  const router = useRouter();
  const { createConversation } = useConversations();

  // A sport as the profile spells it, in the athlete's language when the
  // vocabulary knows it and as spelled on the wire when it does not.
  const sportLabel = (sport: string): string => {
    const key = activitySportLabelKey(sport);
    return key ? t(key) : sport;
  };

  const { data, isLoading, isError } = useQuery({
    queryKey: ['coaches', 'proposal'] as const,
    queryFn: () => coachesApi.getProposal(),
    staleTime: Infinity,
    retry: 1,
  });

  const handleStart = async (coachId: string, coachTitle: string) => {
    setSelecting(coachId);
    try {
      await coachesApi.recordUsage(coachId);
    } catch {
      // Non-fatal: the choice below still opens the coach's thread.
    }
    // « Démarrer » means start talking to this coach: the step is done, and
    // the athlete lands inside a thread bound to the coach rather than on
    // the list. Marking the step first lets the layout leave onboarding.
    await markSeen();
    try {
      const conversation = await createConversation({ coach_id: coachId, title: coachTitle });
      router.push(threadHref(conversation.id));
    } catch {
      // The list still opens; the "+" starts the thread.
    }
  };

  const finish = () => {
    void markSeen();
  };

  if (isLoading) {
    return (
      <Shell>
        <View className="items-center gap-4 py-10">
          <ActivityIndicator size="large" />
          <Text className="text-base text-on-surface font-medium">{t('app.obAnalyzing')}</Text>
          <Text className="text-sm text-on-surface-variant text-center px-6">
            {t('app.obReadingActivities')}
          </Text>
        </View>
      </Shell>
    );
  }

  if (isError || !data) {
    return (
      <Shell>
        <View className="items-center gap-4 py-10">
          <Text className="text-base text-on-surface font-medium text-center">
            {t('app.obNoSuggestions')}
          </Text>
          <Button title={t('app.continue')} onPress={finish} />
        </View>
      </Shell>
    );
  }

  const { profile, coaches } = data;

  return (
    <Shell
      heading={
        user?.display_name
          ? t('app.obStartingLineup', { name: user.display_name })
          : t('frag.startingLineup')
      }
    >
      <View className="mt-5 rounded-2xl border border-outline-variant bg-surface-container px-4 py-4">
        {profile.has_profile ? (
          <>
            <Text className="text-sm text-on-surface">
              {t('app.obWindowSummary', {
                days: profile.window_days,
                count: profile.total_activities,
              })}
              {profile.primary_sport
                ? t('app.obMostlySport', {
                    // The READ side of the value/label split. `primary_sport`
                    // is the wire value — 'Running' — because that is what the
                    // chip saved and what the coach reads. Interpolating it raw
                    // put English inside French prose: ", surtout Running."
                    // A sport the server sends that is not one of ours falls
                    // back to its own name rather than to a missing key.
                    sport: sportLabel(profile.primary_sport),
                  })
                : ''}
            </Text>
            <View className="mt-3 gap-2">
              {profile.sport_mix.map((s) => (
                <View key={s.sport} className="flex-row items-center gap-3">
                  <Text className="w-20 text-xs text-on-surface-variant">{sportLabel(s.sport)}</Text>
                  <View className="flex-1 h-2 rounded-full bg-surface-container-high overflow-hidden">
                    <View
                      className="h-full bg-primary"
                      style={{ width: `${Math.round(s.share * 100)}%` }}
                    />
                  </View>
                  <Text className="w-10 text-right text-xs text-on-surface-variant">
                    {Math.round(s.share * 100)}%
                  </Text>
                </View>
              ))}
            </View>
          </>
        ) : (
          <Text className="text-sm text-on-surface-variant">
            {t('app.obNoActivitiesYet')}
          </Text>
        )}
      </View>

      <View className="mt-5 gap-3">
        {coaches.map((proposed) => (
          <CoachProposalCard
            key={proposed.coach.id}
            proposed={proposed}
            selecting={selecting === proposed.coach.id}
            disabled={selecting !== null}
            onStart={() => void handleStart(proposed.coach.id, proposed.coach.title)}
          />
        ))}
      </View>

      <View className="mt-6">
        <Button title={t('app.skipForNow')} variant="secondary" onPress={finish} disabled={selecting !== null} />
      </View>
    </Shell>
  );
}

function CoachProposalCard({
  proposed,
  selecting,
  disabled,
  onStart,
}: {
  proposed: ProposedCoach;
  selecting: boolean;
  disabled: boolean;
  onStart: () => void;
}) {
  const { t } = useTranslation();
  const { coach, reason } = proposed;
  return (
    <View className="rounded-2xl border border-outline-variant bg-surface-container-low px-4 py-4">
      <View className="flex-row items-start justify-between gap-3">
        <View className="flex-1">
          <Text className="text-base font-semibold text-on-surface" numberOfLines={1}>
            {coach.title}
          </Text>
          <Text className="mt-0.5 text-xs uppercase text-on-surface-variant">
            {t(coachCategoryLabelKey(coach.category))}
          </Text>
        </View>
        <Button
          title={selecting ? t('app.starting') : t('app.start')}
          onPress={onStart}
          disabled={disabled}
        />
      </View>
      {reason ? <Text className="mt-2 text-sm text-on-surface-variant">{reason}</Text> : null}
    </View>
  );
}

function Shell({ heading, children }: { heading?: string; children: React.ReactNode }) {
  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="coach-proposal-screen">
      <ScrollView contentContainerClassName="px-5 py-8">
        <Card>
          <View className="px-2 py-2">
            {heading ? (
              <Text className="text-2xl font-bold text-on-surface text-center">{heading}</Text>
            ) : null}
            {children}
          </View>
        </Card>
      </ScrollView>
    </SafeAreaView>
  );
}
