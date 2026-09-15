// ABOUTME: Memory pane — what the coach remembers, one compact row per fact under a Section per kind, text tabs to filter by kind
// ABOUTME: Forgetting is the long-press menu or the swipe action, never a button on the row (Boreal v2.2 Phase 4, DESIGN.md §10)
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useMemo, useState } from 'react';
import { View, Text, ScrollView, ActivityIndicator, Alert, RefreshControl } from 'react-native';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type { MemoryFactRow } from '@pierre/api-client';
import { MEMORY_KIND_LABEL_KEY, formatNotificationTime } from '@pierre/shared-constants';
import { MEMORY_FACT_KINDS } from '@pierre/shared-types';
import { spacing, useThemeColors } from '../../constants/theme';
import { EmptyState, Row, Section, SwipeableRow, TextTabs, type SwipeAction } from '../../components/ui';
import { userApi } from '../../services/api';
import { Stack } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { presentMemoryFactMenu } from './presentMemoryFactMenu';

const MEMORY_FACTS_QUERY_KEY = ['memory', 'facts'] as const;

/** The tab that stands for "no kind filter"; the server sees `kind: undefined`. */
const ALL_KINDS_TAB = 'all';

export function MemoryScreen(): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const queryClient = useQueryClient();
  const [kindFilter, setKindFilter] = useState<MemoryFactRow['kind'] | ''>('');

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: [...MEMORY_FACTS_QUERY_KEY, kindFilter],
    queryFn: () =>
      userApi.listMemoryFacts({
        kind: kindFilter || undefined,
        limit: 100,
      }),
  });

  const forgetMutation = useMutation({
    mutationFn: (factId: string) => userApi.forgetMemoryFact(factId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: MEMORY_FACTS_QUERY_KEY });
    },
    onError: (err: unknown) => {
      const msg = err instanceof Error ? err.message : String(err);
      Alert.alert(t('app.couldNotForgetFact'), msg);
    },
  });

  const facts = useMemo(() => data?.facts ?? [], [data?.facts]);

  const groupedByKind = useMemo(() => {
    const groups = new Map<MemoryFactRow['kind'], MemoryFactRow[]>();
    for (const f of facts) {
      const bucket = groups.get(f.kind);
      if (bucket) {
        bucket.push(f);
      } else {
        groups.set(f.kind, [f]);
      }
    }
    return Array.from(groups.entries());
  }, [facts]);

  const confirmForget = (fact: MemoryFactRow): void => {
    Alert.alert(
      t('app.forgetThisFactQ'),
      t('app.confirmForgetFact', { fact: fact.sentence }),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.forget'),
          style: 'destructive',
          onPress: () => forgetMutation.mutate(fact.id),
        },
      ],
    );
  };

  // The tabs and the section titles read the same shared table, so a kind the
  // server sends is never a translated word in one place and a raw enum in the other.
  const kindTabs = [
    { key: ALL_KINDS_TAB, label: t('shell.memoryFilterAllKinds') },
    ...MEMORY_FACT_KINDS.map((kind) => ({ key: kind, label: t(MEMORY_KIND_LABEL_KEY[kind]) })),
  ];

  const filtered = kindFilter !== '';

  return (
    <View className="flex-1 bg-background-primary" testID="memory-screen">
      {/* Memory is a settings pane like Notifications or About; the native
          header names it — the same `shell.memoryTitle` the web panel reads —
          and carries the way back. */}
      <Stack.Screen options={{ title: t('shell.memoryTitle') }} />
      <ScrollView
        contentInsetAdjustmentBehavior="automatic"
        contentContainerStyle={{ paddingBottom: spacing.xl }}
        refreshControl={
          <RefreshControl
            refreshing={isFetching}
            onRefresh={() => {
              refetch();
            }}
            tintColor={colors.text.primary}
          />
        }
      >
        <Text className="text-sm text-text-secondary px-4 pt-2 pb-2.5">{t('app.memoryPanelBlurb')}</Text>

        <TextTabs
          testID="memory-kind-tab"
          items={kindTabs}
          value={kindFilter || ALL_KINDS_TAB}
          onChange={(key) => setKindFilter(key === ALL_KINDS_TAB ? '' : (key as MemoryFactRow['kind']))}
        />

        {isLoading ? (
          <View className="py-8 items-center">
            <ActivityIndicator color={colors.text.primary} />
          </View>
        ) : isError ? (
          // The retry is a sibling, not a nested span: Android gives a nested
          // `Text` no native view, so a tap on it would reach nothing there.
          <View className="flex-row flex-wrap items-baseline px-4 py-3" testID="memory-error">
            <Text className="text-sm text-error">
              {t('app.failedLoadMemoryFacts', {
                reason: error instanceof Error ? error.message : String(error),
              })}
            </Text>
            <Text
              className="text-sm text-primary font-medium ml-1"
              accessibilityRole="button"
              onPress={() => {
                refetch();
              }}
              testID="memory-retry"
            >
              {t('common.retry')}
            </Text>
          </View>
        ) : facts.length === 0 ? (
          // The query is filtered server-side, so an empty result under a tab
          // is "none of this type", not "none at all". Telling an athlete who
          // has memory that they have none is a different sentence — and it
          // needs the way back to all types.
          <EmptyState
            testID={filtered ? 'memory-empty-filtered' : 'memory-empty'}
            className="pt-6"
            action={
              filtered
                ? {
                    label: t('shell.memoryShowAllKinds'),
                    onPress: () => setKindFilter(''),
                    testID: 'memory-show-all-kinds',
                  }
                : undefined
            }
          >
            {filtered ? t('shell.memoryEmptyFiltered') : t('shell.memoryEmpty')}
          </EmptyState>
        ) : (
          <View className="gap-8 pt-6">
            {groupedByKind.map(([kind, items]) => (
              <Section
                key={kind}
                testID={`memory-section-${kind}`}
                title={t(MEMORY_KIND_LABEL_KEY[kind])}
                actions={
                  <Text testID="memory-fact-count" className="text-xs font-mono tabular-nums text-text-tertiary">
                    {t(items.length === 1 ? 'shell.memoryFactCountOne' : 'shell.memoryFactCountN', {
                      count: items.length,
                    })}
                  </Text>
                }
              >
                {items.map((fact, idx) => (
                  <FactRow
                    key={fact.id}
                    fact={fact}
                    last={idx === items.length - 1}
                    onForget={() => confirmForget(fact)}
                  />
                ))}
              </Section>
            ))}
          </View>
        )}
      </ScrollView>
    </View>
  );
}

interface FactRowProps {
  fact: MemoryFactRow;
  last: boolean;
  onForget: () => void;
}

/**
 * One remembered fact: the server's sentence as the title, the coach it
 * belongs to under it, and how long ago it was updated in mono on the right.
 * A long-press opens the platform menu whose one row is Forget; a swipe left
 * reveals the same action. Both land on the same confirm.
 */
function FactRow({ fact, last, onForget }: FactRowProps): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();

  const forgetAction: SwipeAction[] = [
    {
      icon: 'trash-2',
      label: t('shell.memoryForget'),
      color: colors.tokens.onError,
      backgroundColor: colors.error,
      onPress: onForget,
    },
  ];

  return (
    <SwipeableRow rightActions={forgetAction} testID={`memory-fact-${fact.id}-swipe`}>
      <View className="bg-background-primary">
        <Row
          compact
          last={last}
          testID={`memory-fact-${fact.id}`}
          title={fact.sentence}
          // The coach is named by title, never by its id — a UUID means nothing to the athlete.
          subtitle={fact.agent_title ? t('shell.memoryFactAgent', { name: fact.agent_title }) : undefined}
          trailing={
            <Text testID="memory-fact-meta" className="text-sm font-mono tabular-nums text-text-secondary">
              {formatNotificationTime(fact.updated_at, t)}
            </Text>
          }
          accessibilityLabel={t('shell.memoryForgetFactLabel', { fact: fact.sentence })}
          onLongPress={() => presentMemoryFactMenu({ onForget }, t)}
        />
      </View>
    </SwipeableRow>
  );
}
