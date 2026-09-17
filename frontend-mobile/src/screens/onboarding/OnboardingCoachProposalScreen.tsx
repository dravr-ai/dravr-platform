// ABOUTME: Post-connect onboarding step (mobile) — "analyzing your data" then inferred profile + top-3 coaches
// ABOUTME: Mirrors the web OnboardingCoachProposal; backed by GET /api/agents/proposal

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useQuery } from '@tanstack/react-query';
import type { ProposedAgent } from '@pierre/shared-types';
import type { OnboardingProgressItem } from '@pierre/shared-constants';
import { Button } from '../../components/ui';
import { OnboardingProgressBar } from '../../components/ui/OnboardingProgressBar';
import { coachesApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useCoachProposalSeen } from '../../hooks/useCoachProposalSeen';
import { useOnboardingProgress } from '../../hooks/useOnboardingProgress';
import { useTranslation } from '@pierre/i18n';
import { activitySportLabelKey, coachCategoryLabelKey } from '@pierre/shared-constants';
import { useRouter } from 'expo-router';
import { useConversations } from '../chat/useConversations';
import { threadHref } from '../../navigation/routes';

/**
 * Onboarding coach proposal (mobile).
 *
 * Reached only via RootLayoutNav when the user has a connected provider but
 * hasn't completed this step. On mount it calls `GET /api/agents/proposal`
 * (activity scan + LLM re-rank) and shows an "analyzing your data" spinner
 * while it runs, then the inferred sport profile and up-to-3 coaches with a
 * rationale each. Completing the step flips the shared `useCoachProposalSeen`
 * cache, which routes the user on to chat.
 */
export function OnboardingCoachProposalScreen() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const { markSeen } = useCoachProposalSeen(user?.id);
  const progress = useOnboardingProgress('coach_proposal');
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

  const handleStart = async (agentId: string) => {
    setSelecting(agentId);
    try {
      await coachesApi.recordUsage(agentId);
    } catch {
      // Non-fatal: the choice below still opens the coach's thread.
    }
    // « Démarrer » means start talking to this coach: the step is done, and
    // the athlete lands inside a thread bound to the coach rather than on
    // the list. Marking the step first lets the layout leave onboarding. The
    // server names the thread after the coach.
    await markSeen();
    try {
      const conversation = await createConversation({ agent_id: agentId });
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
      <Shell progress={progress}>
        <View className="items-center gap-4 py-10">
          <ActivityIndicator size="large" />
          <Text className="text-base text-on-surface font-medium">{t('onboarding.analyzingTrainingData')}</Text>
          <Text className="text-sm text-on-surface-variant text-center px-6">
            {t('onboarding.readingActivitiesHint')}
          </Text>
        </View>
      </Shell>
    );
  }

  if (isError || !data) {
    return (
      <Shell progress={progress}>
        <View className="items-center gap-4 py-10">
          <Text className="text-base text-on-surface font-medium text-center">
            {t('onboarding.agentSuggestionsFailed')}
          </Text>
          <Button title={t('app.continue')} onPress={finish} />
        </View>
      </Shell>
    );
  }

  const { profile, agents } = data;

  return (
    <Shell
      progress={progress}
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
            {t('onboarding.agentProposalNoActivities')}
          </Text>
        )}
      </View>

      <View className="mt-5 gap-3">
        {agents.map((proposed) => (
          <CoachProposalCard
            key={proposed.agent.id}
            proposed={proposed}
            selecting={selecting === proposed.agent.id}
            disabled={selecting !== null}
            onStart={() => void handleStart(proposed.agent.id)}
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
  proposed: ProposedAgent;
  selecting: boolean;
  disabled: boolean;
  onStart: () => void;
}) {
  const { t } = useTranslation();
  const { agent, reason } = proposed;
  return (
    <View className="rounded-2xl border border-outline-variant bg-surface-container-low px-4 py-4">
      <View className="flex-row items-start justify-between gap-3">
        <View className="flex-1">
          <Text className="text-base font-semibold text-on-surface" numberOfLines={1}>
            {agent.title}
          </Text>
          <Text className="mt-0.5 text-xs text-on-surface-variant">
            {t(coachCategoryLabelKey(agent.category))}
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

function Shell({
  heading,
  children,
  progress,
}: {
  heading?: string;
  children: React.ReactNode;
  progress: OnboardingProgressItem[];
}) {
  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="coach-proposal-screen">
      <ScrollView contentContainerClassName="px-5 py-8">
        <OnboardingProgressBar steps={progress} />
        {heading ? (
          <Text className="mt-4 text-3xl font-display text-left text-on-surface">{heading}</Text>
        ) : null}
        {children}
      </ScrollView>
    </SafeAreaView>
  );
}
