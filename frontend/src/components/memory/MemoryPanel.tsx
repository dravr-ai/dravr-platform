// ABOUTME: Phase B Sprint C5 — user-facing memory panel with GDPR Forget per-fact
// ABOUTME: Lists pierre-memory user_facts grouped by kind, lets users forget individual rows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useMemo, useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type { MemoryFactRow } from '@pierre/api-client';
import { MEMORY_KIND_LABEL_KEY } from '@pierre/shared-constants';
import { MEMORY_FACT_KINDS } from '@pierre/shared-types';
import { formatDateTime } from '@pierre/chat-utils';
import { userApi } from '../../services/api';
import { Card, Button, Badge, ConfirmDialog } from '../ui';
import { useTranslation } from '@pierre/i18n';

const MEMORY_FACTS_QUERY_KEY = ['memory', 'facts'] as const;

// The chips and the group badge read the same shared table, so a kind the
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

function factCount(t: (key: string, options: { count: number }) => string, count: number): string {
  return t(count === 1 ? 'shell.memoryFactCountOne' : 'shell.memoryFactCountN', { count });
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

        {/* Chips, the control the phone already uses for this filter. A native
            select sitting between design-system cards paints its own focus
            ring and its own type, and the two clients then answer the same
            question with two different widgets. */}
        <div className="mt-4">
          <p className="text-xs font-medium uppercase tracking-wide text-outline">
            {t('shell.memoryFilterByKind')}
          </p>
          <div
            role="group"
            aria-label={t('shell.memoryFilterByKind')}
            data-testid="memory-kind-filter"
            className="mt-2 flex flex-wrap items-center gap-2"
          >
            {kindOptions(t).map((opt) => {
              const active = kindFilter === opt.value;
              return (
                <button
                  key={opt.value === '' ? 'all' : opt.value}
                  type="button"
                  aria-pressed={active}
                  data-testid={`memory-kind-chip-${opt.value === '' ? 'all' : opt.value}`}
                  onClick={() => setKindFilter(opt.value)}
                  className={`rounded-full border px-3 py-1 text-sm transition-colors ${
                    active
                      ? 'border-primary bg-primary/15 text-on-surface'
                      : 'ghost-border bg-surface-container-low text-on-surface-variant hover:text-on-surface'
                  }`}
                >
                  {opt.label}
                </button>
              );
            })}
            <span data-testid="memory-fact-count" className="ml-1 text-xs text-outline">
              {factCount(t, facts.length)}
            </span>
          </div>
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
        // Two different absences. `facts` is the FILTERED list, so a type with
        // no matches told an athlete who has memory that they have none and
        // invited them to go earn some. The filtered case says what it means
        // and hands back the way out.
        <Card className="p-12 text-center">
          <div data-testid={kindFilter === '' ? 'memory-empty' : 'memory-empty-filtered'}>
            <p className="text-on-surface-variant">
              {kindFilter === '' ? t('shell.memoryEmpty') : t('shell.memoryEmptyFiltered')}
            </p>
            <p className="mt-2 text-xs text-outline">
              {kindFilter === '' ? t('shell.memoryEmptyHint') : t('shell.memoryEmptyFilteredHint')}
            </p>
            {kindFilter === '' ? null : (
              <div className="mt-4">
                <Button
                  variant="secondary"
                  onClick={() => setKindFilter('')}
                  data-testid="memory-show-all-kinds"
                >
                  {t('shell.memoryShowAllKinds')}
                </Button>
              </div>
            )}
          </div>
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
                      {/* The server renders the sentence in the athlete's locale;
                          the panel shows it verbatim so no client grammar exists. */}
                      <p className="text-sm text-on-surface">{fact.sentence}</p>
                      <p data-testid="memory-fact-meta" className="mt-1 text-xs text-on-surface-variant">
                        {t('shell.memoryFactMeta', {
                          confidence: (fact.confidence * 100).toFixed(0),
                          updated: formatDateTime(fact.updated_at, language),
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
          message={t('app.confirmForgetFact', { fact: pendingForget.sentence })}
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
