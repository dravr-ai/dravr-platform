// ABOUTME: The facts the agent inferred and is waiting on the athlete to keep or discard
// ABOUTME: Each draft quotes the words it came from; only a kept fact reaches the agent's prompt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { isSupersededDraftError, type MemoryFactRow } from '@pierre/api-client';
import { formatDateTime } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { userApi } from '../../services/api';
import { Section, Button } from '../ui';

/** Shares the panel's key prefix, so keeping or discarding refreshes both lists. */
export const MEMORY_DRAFTS_QUERY_KEY = ['memory', 'facts', 'drafts'] as const;

export default function MemoryReviewSection() {
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
    onError: (error, factId) => {
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
      title={t('shell.memoryReviewTitle', { count: drafts.length })}
      description={t('shell.memoryReviewBlurb')}
    >
      <ul data-testid="memory-review-list">
        {drafts.map((fact) => (
          <li
            key={fact.id}
            data-testid="memory-review-item"
            className="flex flex-col gap-3 border-t ghost-border-faint py-3 first:border-t-0 sm:flex-row sm:items-start sm:justify-between"
          >
            <div className="min-w-0 flex-1">
              <p className="text-sm text-on-surface">{fact.sentence}</p>
              {fact.evidence ? (
                <blockquote
                  data-testid="memory-review-evidence"
                  className="mt-1 border-l-2 ghost-border pl-3 text-sm italic text-on-surface-variant"
                >
                  “{fact.evidence}”
                </blockquote>
              ) : null}
              <p className="mt-1 text-xs text-outline">{origin(fact)}</p>
              {superseded.has(fact.id) ? (
                <p role="alert" className="mt-1 text-xs text-error">
                  {t('shell.memoryReviewSuperseded')}
                </p>
              ) : null}
            </div>
            <div className="flex shrink-0 gap-2">
              <Button
                variant="tertiary"
                size="sm"
                data-testid="memory-review-discard"
                onClick={() => discard.mutate(fact.id)}
                disabled={busy}
              >
                {t('shell.memoryReviewDiscard')}
              </Button>
              <Button
                variant="primary"
                size="sm"
                data-testid="memory-review-keep"
                onClick={() => keep.mutate(fact.id)}
                disabled={busy || superseded.has(fact.id)}
              >
                {t('shell.memoryReviewKeep')}
              </Button>
            </div>
          </li>
        ))}
      </ul>
    </Section>
  );
}
