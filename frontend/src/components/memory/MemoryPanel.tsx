// ABOUTME: Phase B Sprint C5 — user-facing memory panel with GDPR Forget per-fact
// ABOUTME: Lists pierre-memory user_facts grouped by kind, lets users forget individual rows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useMemo, useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type { MemoryFactRow } from '@pierre/api-client';
import { MEMORY_FACT_KINDS, MEMORY_KIND_LABEL_KEY } from '@pierre/shared-constants';
import { userApi } from '../../services/api';
import { Card, Button, Badge, ConfirmDialog, Select } from '../ui';
import { useTranslation } from '@pierre/i18n';

const MEMORY_FACTS_QUERY_KEY = ['memory', 'facts'] as const;

// The dropdown and the group badge read the same shared table, so a kind the
// server sends is never a translated word in one place and a raw enum in the other.
function kindOptions(t: (key: string) => string): { value: MemoryFactRow['kind'] | ''; label: string }[] {
  return [
    { value: '', label: t('shell.memoryFilterAllKinds') },
    ...MEMORY_FACT_KINDS.map((kind) => ({ value: kind, label: t(MEMORY_KIND_LABEL_KEY[kind]) })),
  ];
}

const KIND_VARIANT: Record<MemoryFactRow['kind'], 'success' | 'info' | 'warning' | 'error' | 'secondary'> = {
  preference: 'info',
  physiology: 'success',
  injury: 'warning',
  goal: 'info',
  schedule: 'secondary',
  equipment: 'secondary',
  north_star: 'info',
  medical: 'warning',
  other: 'secondary',
};

// Formatted in the locale the panel renders in, not the browser's: a French
// athlete reads "1 sept. 2026, 16:28", not "9/1/2026, 4:28:23 PM".
function formatUpdated(iso: string, language: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  return new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' }).format(date);
}

function factCount(t: (key: string, options: { count: number }) => string, count: number): string {
  return t(count === 1 ? 'shell.memoryFactCountOne' : 'shell.memoryFactCountN', { count });
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

export default function MemoryPanel() {
  const { t, language } = useTranslation();
  const queryClient = useQueryClient();
  const [kindFilter, setKindFilter] = useState<MemoryFactRow['kind'] | ''>('');
  const [pendingForget, setPendingForget] = useState<MemoryFactRow | null>(null);

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: [...MEMORY_FACTS_QUERY_KEY, kindFilter],
    queryFn: () =>
      userApi.listMemoryFacts({
        kind: kindFilter || undefined,
        limit: 100,
      }),
  });

  const facts = useMemo(() => data?.facts ?? [], [data?.facts]);

  const forgetMutation = useMutation({
    mutationFn: (factId: string) => userApi.forgetMemoryFact(factId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: MEMORY_FACTS_QUERY_KEY });
      setPendingForget(null);
    },
    onError: () => {
      setPendingForget(null);
    },
  });

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
    return groups;
  }, [facts]);

  return (
    <div className="space-y-4">
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-on-surface">
              {t('shell.memoryTitle')}
            </h2>
            <p className="mt-1 text-sm text-on-surface-variant">
              {t('app.memoryPanelBlurb')}
            </p>
          </div>
          <Button variant="secondary" onClick={() => refetch()}>
            {t('shell.memoryRefresh')}
          </Button>
        </div>

        <div className="mt-4 flex flex-wrap items-end gap-3">
          <div className="w-56">
            <Select
              label={t('shell.memoryFilterByKind')}
              size="sm"
              value={kindFilter}
              onChange={(e) =>
                setKindFilter((e.target.value || '') as MemoryFactRow['kind'] | '')
              }
              options={kindOptions(t).map((opt) => ({ value: opt.value, label: opt.label }))}
            />
          </div>
          <span data-testid="memory-fact-count" className="pb-2 text-xs text-outline">
            {factCount(t, facts.length)}
          </span>
        </div>
      </Card>

      {isLoading ? (
        <Card className="p-12">
          <div className="flex justify-center">
            <div className="pierre-spinner" />
          </div>
        </Card>
      ) : isError ? (
        <Card className="p-6">
          <p className="text-sm text-error">
            {t('frag.failedLoadMemory')}{' '}
            {error instanceof Error ? error.message : String(error)}
          </p>
        </Card>
      ) : facts.length === 0 ? (
        <Card className="p-12 text-center">
          <p className="text-on-surface-variant">
            {t('shell.memoryEmpty')}
          </p>
          <p className="mt-2 text-xs text-outline">
            {t('shell.memoryEmptyHint')}
          </p>
        </Card>
      ) : (
        <div className="space-y-4">
          {Array.from(groupedByKind.entries()).map(([kind, items]) => (
            <Card key={kind} className="overflow-hidden">
              <div className="border-b border-outline-variant bg-surface-container px-4 py-2">
                <div className="flex items-center gap-2">
                  <Badge variant={KIND_VARIANT[kind]}>{t(MEMORY_KIND_LABEL_KEY[kind])}</Badge>
                  <span data-testid="memory-fact-count" className="text-xs text-on-surface-variant">
                    {factCount(t, items.length)}
                  </span>
                </div>
              </div>
              <ul className="divide-y divide-outline-variant">
                {items.map((fact) => (
                  <li key={fact.id} className="flex items-start justify-between gap-4 px-4 py-3">
                    <div className="min-w-0 flex-1">
                      <p className="text-sm text-on-surface">
                        {isUserSubject(fact.subject) ? (
                          <>
                            <span className="text-on-surface-variant">
                              {capitalizeFirst(fact.predicate)}
                            </span>{' '}
                            <span className="font-medium">{fact.object}</span>
                          </>
                        ) : (
                          <>
                            <span className="font-medium">{fact.subject}</span>{' '}
                            <span className="text-on-surface-variant">{fact.predicate}</span>{' '}
                            <span className="font-medium">{fact.object}</span>
                          </>
                        )}
                      </p>
                      <p data-testid="memory-fact-meta" className="mt-1 text-xs text-on-surface-variant">
                        {t('shell.memoryFactMeta', {
                          confidence: (fact.confidence * 100).toFixed(0),
                          updated: formatUpdated(fact.updated_at, language),
                        })}
                        {/* The coach is named by title, never by its id — a UUID means nothing to the athlete. */}
                        {fact.coach_title ? ` · ${t('shell.memoryFactCoach', { name: fact.coach_title })}` : ''}
                      </p>
                    </div>
                    <Button
                      variant="secondary"
                      onClick={() => setPendingForget(fact)}
                      disabled={forgetMutation.isPending}
                    >
                      {t('shell.memoryForget')}
                    </Button>
                  </li>
                ))}
              </ul>
            </Card>
          ))}
        </div>
      )}

      {pendingForget ? (
        <ConfirmDialog
          isOpen
          title={t('shell.memoryForgetConfirm')}
          message={t('app.confirmForgetFact', { fact: factSentence(pendingForget) })}
          confirmLabel={t('app.forget')}
          cancelLabel={t('common.cancel')}
          variant="danger"
          isLoading={forgetMutation.isPending}
          onConfirm={() => forgetMutation.mutate(pendingForget.id)}
          onClose={() => setPendingForget(null)}
        />
      ) : null}
    </div>
  );
}
