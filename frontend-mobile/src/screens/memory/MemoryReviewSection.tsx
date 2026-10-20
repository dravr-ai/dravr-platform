// ABOUTME: The facts the agent inferred and is waiting on the athlete to keep or discard
// ABOUTME: Each draft quotes the words it came from; only a kept fact reaches the agent's prompt
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text } from 'react-native';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { isSupersededDraftError, type MemoryFactRow } from '@pierre/api-client';
import { formatDateTime } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { Button, Section } from '../../components/ui';
import { userApi } from '../../services/api';

/** Shares the screen's key prefix, so keeping or discarding refreshes both lists. */
export const MEMORY_DRAFTS_QUERY_KEY = ['memory', 'facts', 'drafts'] as const;

export function MemoryReviewSection(): React.JSX.Element | null {
  const { t, language } = useTranslation();
  const queryClient = useQueryClient();
  // A draft a newer fact replaced: approving it would restore stale memory.
  const [superseded, setSuperseded] = useState<ReadonlySet<string>>(new Set());

  const { data } = useQuery({
    queryKey: MEMORY_DRAFTS_QUERY_KEY,
    queryFn: () => userApi.listMemoryFacts({ status: 'draft', limit: 100 }),
  });
  const drafts = data?.facts ?? [];

  const refresh = () => queryClient.invalidateQueries({ queryKey: ['memory', 'facts'] });

  const keep = useMutation({
    mutationFn: (factId: string) => userApi.approveMemoryFact(factId),
    onSuccess: refresh,
    onError: (error: unknown, factId: string) => {
      if (isSupersededDraftError(error)) {
        setSuperseded((ids) => new Set(ids).add(factId));
      }
    },
  });
  const discard = useMutation({
    mutationFn: (factId: string) => userApi.forgetMemoryFact(factId),
    onSuccess: refresh,
  });
  const busy = keep.isPending || discard.isPending;

  if (drafts.length === 0) return null;

  const origin = (fact: MemoryFactRow) =>
    t(fact.evidence_source === 'agent' ? 'shell.memoryReviewFromAgent' : 'shell.memoryReviewFromAthlete', {
      date: formatDateTime(fact.updated_at, language),
    });

  return (
    <Section
      className="pt-6"
      testID="memory-review"
      title={t('shell.memoryReviewTitle', { count: drafts.length })}
      description={t('shell.memoryReviewBlurb')}
    >
      {drafts.map((fact, idx) => (
        <View
          key={fact.id}
          testID={`memory-review-${fact.id}`}
          className={`px-4 py-3 gap-1 ${idx === 0 ? '' : 'border-t border-border-faint'}`}
        >
          <Text className="text-base text-text-primary">{fact.sentence}</Text>
          {fact.evidence ? (
            <Text
              testID={`memory-review-${fact.id}-evidence`}
              className="text-sm italic text-text-secondary pl-3 border-l border-border-faint"
            >
              “{fact.evidence}”
            </Text>
          ) : null}
          <Text className="text-xs text-text-tertiary">{origin(fact)}</Text>
          {superseded.has(fact.id) ? (
            <Text accessibilityRole="alert" className="text-xs text-error">
              {t('shell.memoryReviewSuperseded')}
            </Text>
          ) : null}
          <View className="flex-row justify-end gap-2 pt-2">
            <Button
              variant="ghost"
              title={t('shell.memoryReviewDiscard')}
              testID={`memory-review-${fact.id}-discard`}
              onPress={() => discard.mutate(fact.id)}
              disabled={busy}
            />
            <Button
              variant="primary"
              title={t('shell.memoryReviewKeep')}
              testID={`memory-review-${fact.id}-keep`}
              onPress={() => keep.mutate(fact.id)}
              disabled={busy || superseded.has(fact.id)}
            />
          </View>
        </View>
      ))}
    </Section>
  );
}
