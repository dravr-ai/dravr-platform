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
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { LinearGradient } from 'expo-linear-gradient';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Feather } from '@expo/vector-icons';
import type { MemoryFactRow } from '@pierre/api-client';
import { formatDateTime } from '@pierre/chat-utils';
import { MEMORY_KIND_LABEL_KEY } from '@pierre/shared-constants';
import { MEMORY_FACT_KINDS } from '@pierre/shared-types';
import { spacing, borderRadius, fontSize, fontWeight, useThemeColors } from '../../constants/theme';
import { userApi } from '../../services/api';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';

const MEMORY_FACTS_QUERY_KEY = ['memory', 'facts'] as const;

export function MemoryScreen(): React.JSX.Element {
  const { t, language } = useTranslation();
  const router = useRouter();
  const colors = useThemeColors();
  const queryClient = useQueryClient();
  const [kindFilter, setKindFilter] = useState<MemoryFactRow['kind'] | ''>('');
  // Whether the chip row still has chips off the right edge. Measured rather
  // than assumed: nine kinds fit on a tablet and overflow a phone, so a fade
  // painted unconditionally would sit over nothing on the wider one.
  const [chipsViewportWidth, setChipsViewportWidth] = useState(0);
  const [chipsContentWidth, setChipsContentWidth] = useState(0);
  const [chipsScrollX, setChipsScrollX] = useState(0);
  const chipsHaveMoreRight = chipsContentWidth - chipsViewportWidth - chipsScrollX > 1;

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
    <SafeAreaView
      style={{ flex: 1, backgroundColor: colors.background.primary }}
      edges={['top']}
      testID="memory-screen"
    >
      {/* Memory is a settings pane like Notifications or About, and carries the
          same way back: it is reached by a push from the settings list, where
          the tab bar is not rendered. */}
      <View
        style={{
          flexDirection: 'row',
          alignItems: 'center',
          paddingHorizontal: spacing.md,
          paddingVertical: spacing.sm,
        }}
      >
        <TouchableOpacity
          onPress={() => router.back()}
          testID="back-button"
          accessibilityRole="button"
          accessibilityLabel={t('common.back')}
          style={{ padding: 8, marginRight: 8 }}
        >
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text style={{ fontSize: 20, fontWeight: '600', color: colors.text.primary }}>
          {t('shell.memoryTitle')}
        </Text>
      </View>

      <ScrollView
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
          <Text style={{ fontSize: fontSize.sm, color: colors.text.secondary }}>
            {t('app.memoryPanelBlurb')}
          </Text>
        </View>

        {/* The row scrolls, and the chip at the right edge is cut mid-word when
            it does. A cut with nothing over it reads as a rendering fault, so
            the fade sits on that edge for exactly as long as there is more to
            reach — it disappears once the last chip is in view. */}
        <View style={{ marginBottom: spacing.md }}>
          <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            scrollEventThrottle={16}
            onLayout={(e) => setChipsViewportWidth(e.nativeEvent.layout.width)}
            onContentSizeChange={(width) => setChipsContentWidth(width)}
            onScroll={(e) => setChipsScrollX(e.nativeEvent.contentOffset.x)}
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
                  style={{
                    paddingHorizontal: spacing.md,
                    paddingVertical: spacing.sm,
                    borderRadius: borderRadius.full,
                    backgroundColor: active
                      ? colors.pierre.violet
                      : 'rgba(255,255,255,0.08)',
                    borderWidth: 1,
                    borderColor: active
                      ? colors.pierre.violet
                      : 'rgba(255,255,255,0.15)',
                  }}
                >
                  <Text
                    style={{
                      color: active ? '#ffffff' : colors.text.secondary,
                      fontSize: fontSize.sm,
                      fontWeight: fontWeight.medium,
                    }}
                  >
                    {opt.label}
                  </Text>
                </TouchableOpacity>
              );
            })}
          </ScrollView>
          {chipsHaveMoreRight ? (
            <LinearGradient
              testID="memory-kind-scroll-fade"
              pointerEvents="none"
              colors={[`${colors.background.primary}00`, colors.background.primary]}
              start={{ x: 0, y: 0.5 }}
              end={{ x: 1, y: 0.5 }}
              style={{
                position: 'absolute',
                right: 0,
                top: 0,
                bottom: 0,
                width: spacing.xl,
              }}
            />
          ) : null}
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
              style={{
                color: colors.text.tertiary,
                marginTop: spacing.xs,
                textAlign: 'center',
                fontSize: fontSize.xs,
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
                <Text style={{ color: colors.pierre.violet, fontSize: fontSize.sm }}>
                  {t('shell.memoryShowAllKinds')}
                </Text>
              </TouchableOpacity>
            )}
          </View>
        ) : (
          groupedByKind.map(([kind, items]) => (
            <View
              key={kind}
              style={{
                backgroundColor: 'rgba(255,255,255,0.04)',
                borderRadius: borderRadius.lg,
                borderWidth: 1,
                borderColor: 'rgba(255,255,255,0.08)',
                marginBottom: spacing.md,
                overflow: 'hidden',
              }}
            >
              <View
                style={{
                  paddingHorizontal: spacing.md,
                  paddingVertical: spacing.sm,
                  backgroundColor: 'rgba(255,255,255,0.06)',
                  flexDirection: 'row',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                }}
              >
                <Text
                  style={{
                    color: colors.text.primary,
                    fontSize: fontSize.sm,
                    fontWeight: fontWeight.semibold,
                  }}
                >
                  {t(MEMORY_KIND_LABEL_KEY[kind])}
                </Text>
                <Text
                  testID="memory-fact-count"
                  style={{ color: colors.text.tertiary, fontSize: fontSize.xs }}
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
                    borderTopWidth: idx === 0 ? 0 : 1,
                    borderTopColor: 'rgba(255,255,255,0.06)',
                    flexDirection: 'row',
                    alignItems: 'flex-start',
                    justifyContent: 'space-between',
                    gap: spacing.sm,
                  }}
                >
                  <View style={{ flex: 1 }}>
                    <Text
                      style={{
                        color: colors.text.primary,
                        fontSize: fontSize.sm,
                      }}
                    >
                      {fact.sentence}
                    </Text>
                    <Text
                      testID="memory-fact-meta"
                      style={{
                        color: colors.text.tertiary,
                        fontSize: fontSize.xs,
                        marginTop: spacing.xs,
                      }}
                    >
                      {t('shell.memoryFactMeta', {
                        confidence: (fact.confidence * 100).toFixed(0),
                        updated: formatDateTime(fact.updated_at, language),
                      })}
                      {/* The coach is named by title, never by its id — a UUID means nothing to the athlete. */}
                      {fact.coach_title ? ` · ${t('shell.memoryFactCoach', { name: fact.coach_title })}` : ''}
                    </Text>
                  </View>
                  <TouchableOpacity
                    accessibilityRole="button"
                    accessibilityLabel={t('shell.memoryForgetFactLabel', { fact: fact.sentence })}
                    onPress={() => handleForget(fact)}
                    disabled={forgetMutation.isPending}
                    style={{
                      padding: spacing.sm,
                      borderRadius: borderRadius.md,
                      backgroundColor: 'rgba(255,64,64,0.12)',
                    }}
                  >
                    <Feather
                      name="trash-2"
                      size={16}
                      color={colors.pierre.red}
                    />
                  </TouchableOpacity>
                </View>
              ))}
            </View>
          ))
        )}
      </ScrollView>
    </SafeAreaView>
  );
}
