// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The human coach's reply review on the phone — every reply the agent sent into a group they run, newest first, narrowed to one athlete
// ABOUTME: The header line counts this week's flags and corrections; replies page in with "Show older replies", as on the web (carnet#1304)

import React, { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, RefreshControl, ScrollView, Text, View } from 'react-native';
import { Stack, useLocalSearchParams } from 'expo-router';
import { coachReviewWeekSummary } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Button, EmptyState, TextTabs } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useCoachReplies } from '../../hooks/useCoachReplies';
import { ReplyCard } from './ReplyCard';

/** The filter tab that means every athlete. */
const ALL_ATHLETES = 'all';

export function CoachRepliesScreen() {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const params = useLocalSearchParams<{ athleteId?: string }>();
  const [athleteId, setAthleteId] = useState<string | null>(params.athleteId ?? null);
  const review = useCoachReplies(athleteId);
  const [refreshing, setRefreshing] = useState(false);
  const { refetch } = review;
  const onRefresh = useCallback(() => {
    setRefreshing(true);
    void refetch().finally(() => setRefreshing(false));
  }, [refetch]);

  const { first, replies } = review;
  // The filter keeps its choices while another athlete's page loads, so the
  // row does not vanish and come back under the coach's thumb.
  const [choices, setChoices] = useState(first?.athletes);
  useEffect(() => {
    if (first !== undefined) setChoices(first.athletes);
  }, [first]);
  const weekLine = review.thisWeek ? coachReviewWeekSummary(t, review.thisWeek, language) : null;

  let body: React.ReactNode;
  if (review.isLoading) {
    body = (
      <View className="items-center py-10" accessibilityLabel={t('common.loading')} testID="coach-replies-loading">
        <ActivityIndicator color={colors.tokens.primary} />
      </View>
    );
  } else if (review.isError) {
    body = (
      <EmptyState testID="coach-replies-failed" action={{ label: t('common.retry'), onPress: refetch, testID: 'coach-replies-retry' }}>
        {t('coachReview.loadFailed')}
      </EmptyState>
    );
  } else if (first !== undefined && first.groups.length === 0) {
    body = <EmptyState testID="coach-replies-not-a-coach">{t('coachReview.notACoach')}</EmptyState>;
  } else if (replies.length === 0) {
    body = <EmptyState testID="coach-replies-empty">{t('coachReview.empty')}</EmptyState>;
  } else {
    body = (
      <View testID="coach-replies-list">
        {replies.map((reply) => (
          <ReplyCard key={reply.id} reply={reply} />
        ))}
        {review.hasNextPage && (
          <View className="items-center py-4">
            <Button
              title={t('coachReview.loadOlder')}
              variant="ghost"
              loading={review.isFetchingNextPage}
              onPress={review.fetchNextPage}
              testID="coach-replies-older"
            />
          </View>
        )}
      </View>
    );
  }

  return (
    <View className="flex-1 bg-background-primary" testID="coach-replies">
      <Stack.Screen options={{ title: t('coachReview.nav') }} />
      <ScrollView
        contentInsetAdjustmentBehavior="automatic"
        contentContainerClassName="pb-8"
        keyboardShouldPersistTaps="handled"
        refreshControl={<RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.tokens.primary} />}
        testID="coach-replies-scroll"
      >
        <View className="gap-1 px-4 pb-3 pt-2">
          <Text accessibilityRole="header" className="font-display text-xl font-semibold text-text-primary">
            {t('coachReview.title')}
          </Text>
          {weekLine !== null && (
            <Text className="text-sm text-text-secondary" testID="coach-replies-week">
              {weekLine}
            </Text>
          )}
        </View>
        {choices !== undefined && choices.length > 0 && (
          <TextTabs
            items={[
              { key: ALL_ATHLETES, label: t('coachReview.allAthletes') },
              ...choices.map((athlete) => ({
                key: athlete.user_id,
                label: athlete.shares_with_coach
                  ? (athlete.name ?? t('coachReview.unnamedAthlete'))
                  : t('coachReview.notSharing', { name: athlete.name ?? t('coachReview.unnamedAthlete') }),
              })),
            ]}
            value={athleteId ?? ALL_ATHLETES}
            onChange={(key) => setAthleteId(key === ALL_ATHLETES ? null : key)}
            testID="coach-replies-filter"
          />
        )}
        {body}
      </ScrollView>
    </View>
  );
}
