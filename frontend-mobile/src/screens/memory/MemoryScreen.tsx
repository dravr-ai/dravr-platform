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
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Feather } from '@expo/vector-icons';
import type { MemoryFactRow } from '@pierre/api-client';
import { MEMORY_FACT_KINDS, MEMORY_KIND_LABEL_KEY } from '@pierre/shared-constants';
import { spacing, borderRadius, fontSize, fontWeight, useThemeColors } from '../../constants/theme';
import { userApi } from '../../services/api';
import { useTranslation } from '@pierre/i18n';

const MEMORY_FACTS_QUERY_KEY = ['memory', 'facts'] as const;

// Formatted in the locale the screen renders in, not the device's: a French
// athlete reads "1 sept. 2026, 16:28", not "9/1/2026, 4:28:23 PM".
function formatUpdated(iso: string, language: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  return new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' }).format(date);
}

// The memory-extraction prompt models predicates as third-person verbs
// ("has", "is", "wants"), so a literal {subject} {predicate} {object}
// render produces "you has connected WHOOP". When the subject is the
// "you" pronoun we drop it and capitalize the predicate so the line
// reads as a sentence the user already knows is about themselves.
function isUserSubject(subject: string): boolean {
  return subject.trim().toLowerCase() === 'you';
}

function capitalizeFirst(text: string): string {
  return text.length === 0 ? text : text.charAt(0).toUpperCase() + text.slice(1);
}

function factSentence(fact: Pick<MemoryFactRow, 'subject' | 'predicate' | 'object'>): string {
  if (isUserSubject(fact.subject)) {
    return `${capitalizeFirst(fact.predicate)} ${fact.object}`.trim();
  }
  return `${fact.subject} ${fact.predicate} ${fact.object}`.trim();
}

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
      t('app.confirmForgetFact', { fact: factSentence(fact) }),
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
      testID="memory-screen"
    >
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
          <Text
            style={{
              fontSize: fontSize.xl,
              fontWeight: fontWeight.bold,
              color: colors.text.primary,
              marginBottom: spacing.xs,
            }}
          >
            {t('app.whatCoachRemembers')}
          </Text>
          <Text style={{ fontSize: fontSize.sm, color: colors.text.secondary }}>
            {t('app.memoryBlurb')}
          </Text>
        </View>

        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          style={{ marginBottom: spacing.md }}
          contentContainerStyle={{ gap: spacing.sm }}
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
          <View
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
              {t('app.noFactsYet')}
            </Text>
            <Text
              style={{
                color: colors.text.tertiary,
                marginTop: spacing.xs,
                textAlign: 'center',
                fontSize: fontSize.xs,
              }}
            >
              {t('app.memoryEmptyBlurb')}
            </Text>
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
                      {isUserSubject(fact.subject) ? (
                        <>
                          {capitalizeFirst(fact.predicate)}{' '}
                          <Text style={{ fontWeight: fontWeight.semibold }}>
                            {fact.object}
                          </Text>
                        </>
                      ) : (
                        <>
                          <Text style={{ fontWeight: fontWeight.semibold }}>
                            {fact.subject}
                          </Text>{' '}
                          {fact.predicate}{' '}
                          <Text style={{ fontWeight: fontWeight.semibold }}>
                            {fact.object}
                          </Text>
                        </>
                      )}
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
                        updated: formatUpdated(fact.updated_at, language),
                      })}
                      {/* The coach is named by title, never by its id — a UUID means nothing to the athlete. */}
                      {fact.coach_title ? ` · ${t('shell.memoryFactCoach', { name: fact.coach_title })}` : ''}
                    </Text>
                  </View>
                  <TouchableOpacity
                    accessibilityRole="button"
                    accessibilityLabel={t('shell.memoryForgetFactLabel', { fact: factSentence(fact) })}
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
