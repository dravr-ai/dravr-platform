// ABOUTME: Phase B Sprint C12 — mobile port of the web MemoryPanel
// ABOUTME: Lists pierre-memory user_facts grouped by kind with per-row forget action
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useMemo, useState } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  RefreshControl,
  StyleSheet,
} from 'react-native';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Feather } from '@expo/vector-icons';
import type { MemoryFactRow } from '@pierre/api-client';
import { formatDateTime } from '@pierre/chat-utils';
import { MEMORY_KIND_LABEL_KEY } from '@pierre/shared-constants';
import { MEMORY_FACT_KINDS } from '@pierre/shared-types';
import { spacing, borderRadius, useThemeColors } from '../../constants/theme';
import { userApi } from '../../services/api';
import { Stack } from 'expo-router';
import { useTranslation } from '@pierre/i18n';

const MEMORY_FACTS_QUERY_KEY = ['memory', 'facts'] as const;

export function MemoryScreen(): React.JSX.Element {
  const { t, language } = useTranslation();
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

  const handleForget = (fact: MemoryFactRow): void => {
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

  // The chips and the group headers read the same shared table, so a kind the
  // server sends is never a translated word in one place and a raw enum in the other.
  const kindOptions: { value: MemoryFactRow['kind'] | ''; label: string }[] = [
    { value: '', label: t('shell.memoryFilterAllKinds') },
    ...MEMORY_FACT_KINDS.map((kind) => ({ value: kind, label: t(MEMORY_KIND_LABEL_KEY[kind]) })),
  ];

  return (
    <View style={{ flex: 1, backgroundColor: colors.background.primary }} testID="memory-screen">
      {/* Memory is a settings pane like Notifications or About; the native
          header names it — the same `shell.memoryTitle` the web panel reads —
          and carries the way back. */}
      <Stack.Screen options={{ title: t('shell.memoryTitle') }} />
      <ScrollView
        contentInsetAdjustmentBehavior="automatic"
        contentContainerStyle={{ padding: spacing.lg }}
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
        <View style={{ marginBottom: spacing.lg }}>
          <Text className="text-sm" style={{ color: colors.text.secondary }}>
            {t('app.memoryPanelBlurb')}
          </Text>
        </View>

        {/* Nine kinds fit on a tablet and overflow a phone, so the chip row
            scrolls; the chip cut at the right edge is what says there is more. */}
        <View style={{ marginBottom: spacing.md }}>
          <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            contentContainerStyle={{ gap: spacing.sm, paddingRight: spacing.lg }}
          >
            {kindOptions.map((opt) => {
              const active = kindFilter === opt.value;
              return (
                <TouchableOpacity
                  key={opt.value || 'all'}
                  onPress={() => setKindFilter(opt.value)}
                  accessibilityRole="button"
                  accessibilityState={{ selected: active }}
                  className={active ? 'bg-primary' : 'bg-surface-container'}
                  style={{
                    paddingHorizontal: spacing.md,
                    paddingVertical: spacing.sm,
                    borderRadius: borderRadius.full,
                    borderWidth: 1,
                    borderColor: active ? colors.tokens.primary : colors.border.default,
                  }}
                >
                  <Text
                    className="text-sm font-medium"
                    style={{ color: active ? colors.tokens.onPrimary : colors.text.secondary }}
                  >
                    {opt.label}
                  </Text>
                </TouchableOpacity>
              );
            })}
          </ScrollView>
        </View>

        {isLoading ? (
          <View style={{ paddingVertical: spacing.xl, alignItems: 'center' }}>
            <ActivityIndicator color={colors.text.primary} />
          </View>
        ) : isError ? (
          <View style={{ paddingVertical: spacing.lg }}>
            <Text style={{ color: colors.pierre.red }}>
              {t('app.failedLoadMemoryFacts', {
                reason: error instanceof Error ? error.message : String(error),
              })}
            </Text>
          </View>
        ) : facts.length === 0 ? (
          // The query is filtered server-side, so an empty result under a chip
          // is "none of this type", not "none at all". Telling an athlete who
          // has memory that they have none, and inviting them to go earn some,
          // is a different sentence — and it needs the way back to all types.
          <View
            testID={kindFilter === '' ? 'memory-empty' : 'memory-empty-filtered'}
            style={{
              paddingVertical: spacing.xl,
              alignItems: 'center',
            }}
          >
            <Feather name="inbox" size={48} color={colors.text.tertiary} />
            <Text
              style={{
                color: colors.text.secondary,
                marginTop: spacing.sm,
                textAlign: 'center',
              }}
            >
              {kindFilter === '' ? t('shell.memoryEmpty') : t('shell.memoryEmptyFiltered')}
            </Text>
            <Text
              className="text-xs"
              style={{
                color: colors.text.tertiary,
                marginTop: spacing.xs,
                textAlign: 'center',
              }}
            >
              {kindFilter === '' ? t('shell.memoryEmptyHint') : t('shell.memoryEmptyFilteredHint')}
            </Text>
            {kindFilter === '' ? null : (
              <TouchableOpacity
                accessibilityRole="button"
                testID="memory-show-all-kinds"
                onPress={() => setKindFilter('')}
                style={{
                  marginTop: spacing.md,
                  paddingHorizontal: spacing.md,
                  paddingVertical: spacing.sm,
                  borderRadius: borderRadius.full,
                  borderWidth: 1,
                  borderColor: colors.pierre.violet,
                }}
              >
                <Text className="text-sm" style={{ color: colors.pierre.violet }}>
                  {t('shell.memoryShowAllKinds')}
                </Text>
              </TouchableOpacity>
            )}
          </View>
        ) : (
          groupedByKind.map(([kind, items]) => (
            <View
              key={kind}
              className="bg-surface-container-low"
              style={{
                borderRadius: borderRadius.lg,
                borderWidth: StyleSheet.hairlineWidth,
                borderColor: colors.border.faint,
                marginBottom: spacing.md,
                overflow: 'hidden',
              }}
            >
              <View
                className="bg-surface-container"
                style={{
                  paddingHorizontal: spacing.md,
                  paddingVertical: spacing.sm,
                  flexDirection: 'row',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                }}
              >
                <Text className="text-sm font-semibold" style={{ color: colors.text.primary }}>
                  {t(MEMORY_KIND_LABEL_KEY[kind])}
                </Text>
                <Text
                  testID="memory-fact-count"
                  className="text-xs font-mono tabular-nums"
                  style={{ color: colors.text.tertiary }}
                >
                  {t(items.length === 1 ? 'shell.memoryFactCountOne' : 'shell.memoryFactCountN', {
                    count: items.length,
                  })}
                </Text>
              </View>
              {items.map((fact, idx) => (
                <View
                  key={fact.id}
                  testID={`memory-fact-${fact.id}`}
                  style={{
                    paddingHorizontal: spacing.md,
                    paddingVertical: spacing.md,
                    borderTopWidth: idx === 0 ? 0 : StyleSheet.hairlineWidth,
                    borderTopColor: colors.border.faint,
                    flexDirection: 'row',
                    alignItems: 'flex-start',
                    justifyContent: 'space-between',
                    gap: spacing.sm,
                  }}
                >
                  <View style={{ flex: 1 }}>
                    <Text className="text-sm" style={{ color: colors.text.primary }}>
                      {fact.sentence}
                    </Text>
                    <Text
                      testID="memory-fact-meta"
                      className="text-xs"
                      style={{ color: colors.text.tertiary, marginTop: spacing.xs }}
                    >
                      {t('shell.memoryFactMeta', {
                        confidence: (fact.confidence * 100).toFixed(0),
                        updated: formatDateTime(fact.updated_at, language),
                      })}
                      {/* The coach is named by title, never by its id — a UUID means nothing to the athlete. */}
                      {fact.agent_title ? ` · ${t('shell.memoryFactAgent', { name: fact.agent_title })}` : ''}
                    </Text>
                  </View>
                  <TouchableOpacity
                    accessibilityRole="button"
                    accessibilityLabel={t('shell.memoryForgetFactLabel', { fact: fact.sentence })}
                    onPress={() => handleForget(fact)}
                    disabled={forgetMutation.isPending}
                    className="bg-error/15"
                    style={{
                      padding: spacing.sm,
                      borderRadius: borderRadius.md,
                    }}
                  >
                    <Feather
                      name="trash-2"
                      size={16}
                      color={colors.error}
                    />
                  </TouchableOpacity>
                </View>
              ))}
            </View>
          ))
        )}
      </ScrollView>
    </View>
  );
}
