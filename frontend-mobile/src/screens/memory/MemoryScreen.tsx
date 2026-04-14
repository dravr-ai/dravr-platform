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
import { colors, spacing, borderRadius, fontSize, fontWeight } from '../../constants/theme';
import { userApi } from '../../services/api';

const MEMORY_FACTS_QUERY_KEY = ['memory', 'facts'] as const;

const KIND_LABELS: Record<string, string> = {
  preference: 'Preferences',
  physiology: 'Physiology',
  injury: 'Injuries',
  goal: 'Goals',
  schedule: 'Schedules',
  equipment: 'Equipment',
  other: 'Other',
};

function humanizeKind(kind: string): string {
  return KIND_LABELS[kind] ?? kind.charAt(0).toUpperCase() + kind.slice(1);
}

function formatTimestamp(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

export function MemoryScreen(): React.JSX.Element {
  const queryClient = useQueryClient();
  const [kindFilter, setKindFilter] = useState<string>('');

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: [...MEMORY_FACTS_QUERY_KEY, kindFilter],
    queryFn: () =>
      userApi.listMemoryFacts({
        kind: (kindFilter || undefined) as MemoryFactRow['kind'] | undefined,
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
      Alert.alert('Could not forget fact', msg);
    },
  });

  const facts = useMemo(() => data?.facts ?? [], [data?.facts]);

  const groupedByKind = useMemo(() => {
    const groups = new Map<string, MemoryFactRow[]>();
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
      'Forget this fact?',
      `The coach will stop using "${fact.subject} ${fact.predicate} ${fact.object}" on the next turn. This cannot be undone.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Forget',
          style: 'destructive',
          onPress: () => forgetMutation.mutate(fact.id),
        },
      ],
    );
  };

  const kindOptions: { value: string; label: string }[] = [
    { value: '', label: 'All' },
    { value: 'preference', label: 'Preferences' },
    { value: 'physiology', label: 'Physiology' },
    { value: 'injury', label: 'Injuries' },
    { value: 'goal', label: 'Goals' },
    { value: 'schedule', label: 'Schedules' },
    { value: 'equipment', label: 'Equipment' },
    { value: 'other', label: 'Other' },
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
            What the coach remembers about you
          </Text>
          <Text style={{ fontSize: fontSize.sm, color: colors.text.secondary }}>
            Facts the platform extracted from your conversations. Forget any
            one and the coach stops using it next turn.
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
              Failed to load memory facts:{' '}
              {error instanceof Error ? error.message : String(error)}
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
              No facts stored yet.
            </Text>
            <Text
              style={{
                color: colors.text.tertiary,
                marginTop: spacing.xs,
                textAlign: 'center',
                fontSize: fontSize.xs,
              }}
            >
              Talk to a coach for a few turns and the platform will start
              building your memory profile.
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
                  {humanizeKind(kind)}
                </Text>
                <Text
                  style={{ color: colors.text.tertiary, fontSize: fontSize.xs }}
                >
                  {items.length} fact{items.length === 1 ? '' : 's'}
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
                      <Text style={{ fontWeight: fontWeight.semibold }}>
                        {fact.subject}
                      </Text>{' '}
                      {fact.predicate}{' '}
                      <Text style={{ fontWeight: fontWeight.semibold }}>
                        {fact.object}
                      </Text>
                    </Text>
                    <Text
                      style={{
                        color: colors.text.tertiary,
                        fontSize: fontSize.xs,
                        marginTop: spacing.xs,
                      }}
                    >
                      Confidence {(fact.confidence * 100).toFixed(0)}% ·
                      Updated {formatTimestamp(fact.updated_at)}
                    </Text>
                  </View>
                  <TouchableOpacity
                    accessibilityRole="button"
                    accessibilityLabel={`Forget ${fact.subject} ${fact.predicate} ${fact.object}`}
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
