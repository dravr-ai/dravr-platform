// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace's Historique tab on the phone — what changed in the plan, newest first, each « when · who · what »
// ABOUTME: A page at a time; « Afficher les changements plus anciens » reads the page before the oldest change shown (carnet#1818)

import React from 'react';
import { ActivityIndicator, ScrollView, Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import { PLAN_WORKSPACE_KEYS, planHistoryLine } from '@pierre/ui-logic';
import { Button, EmptyState } from '../../../components/ui';
import { useThemeColors } from '../../../constants/theme';
import { useSubjectPlanHistory } from '../../../hooks/useSessionEditor';

interface WorkspaceHistoryProps {
  /** The coach's plan thread on the athlete; `null` while it is found or opened. */
  planThreadId: string | null;
  /** The name the thread's agent goes by, for the changes it made. */
  agentName: string;
}

/**
 * Historique: the plan's changes, newest first, a page at a time. The button
 * under the last line reads the page before the oldest change shown and adds
 * it below; it goes once a page comes back short, and a page that could not
 * be read says so and leaves the button to try again.
 */
export function WorkspaceHistory({ planThreadId, agentName }: WorkspaceHistoryProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const history = useSubjectPlanHistory(planThreadId);
  const events = history.data?.pages.flatMap((page) => page.events);

  let body: React.ReactNode;
  if (planThreadId === null || (events === undefined && !history.isError)) {
    body = (
      <View className="items-start px-4 py-3" accessibilityLabel={t('common.loading')} testID="workspace-history-loading">
        <ActivityIndicator color={colors.tokens.primary} />
      </View>
    );
  } else if (events === undefined) {
    body = (
      <EmptyState
        action={{ label: t('common.retry'), onPress: () => void history.refetch(), testID: 'workspace-history-retry' }}
        testID="workspace-history-failed"
      >
        {t(PLAN_WORKSPACE_KEYS.historyFailed)}
      </EmptyState>
    );
  } else if (events.length === 0) {
    body = <EmptyState testID="workspace-history-empty">{t(PLAN_WORKSPACE_KEYS.historyEmpty)}</EmptyState>;
  } else {
    body = (
      <View className="gap-3.5 px-4 pt-3" testID="workspace-history">
        {events.map((event) => {
          const line = planHistoryLine(event, { t, language, agentName });
          return (
            <View
              key={`${event.created_at}-${event.kind}-${event.dates.join(',')}`}
              accessible
              className="flex-row gap-2.5"
              testID="workspace-history-event"
            >
              <Text className="w-14 pt-px font-mono text-xs tabular-nums text-text-tertiary">{line.when}</Text>
              <View className="min-w-0 flex-1">
                <Text className="text-sm font-semibold text-text-primary">{line.who}</Text>
                <Text className="text-sm text-text-secondary">{line.what}</Text>
              </View>
            </View>
          );
        })}
        {history.isFetchNextPageError ? (
          <Text accessibilityRole="alert" className="text-sm text-error" testID="workspace-history-more-failed">
            {t(PLAN_WORKSPACE_KEYS.historyFailed)}
          </Text>
        ) : null}
        {history.hasNextPage ? (
          <View className="self-start">
            <Button
              title={t(PLAN_WORKSPACE_KEYS.historyMore)}
              variant="ghost"
              loading={history.isFetchingNextPage}
              onPress={() => void history.fetchNextPage()}
              testID="workspace-history-more"
            />
          </View>
        ) : null}
      </View>
    );
  }

  return (
    <ScrollView contentContainerClassName="pb-10 pt-3" testID="workspace-history-tab">
      <Text className="px-4 text-sm text-text-secondary">{t(PLAN_WORKSPACE_KEYS.historyIntro)}</Text>
      {body}
    </ScrollView>
  );
}
