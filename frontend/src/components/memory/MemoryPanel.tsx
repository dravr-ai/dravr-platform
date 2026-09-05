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
import { Section, Button, Badge, ConfirmDialog } from '../ui';
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
    <div className="space-y-8">
      <Section
        title={t('shell.memoryTitle')}
        description={t('app.memoryPanelBlurb')}
        actions={
          <Button variant="tertiary" size="sm" onClick={() => refetch()}>
            {t('shell.memoryRefresh')}
          </Button>
        }
      >

        {/* Text tabs, the filter language of every athlete surface (DESIGN.md
            §5): sentence-case words under a primary underline, the count in
            mono beside them. The phone keeps its chips for now. */}
        <div>
          <div
            role="group"
            aria-label={t('shell.memoryFilterByKind')}
            data-testid="memory-kind-filter"
            className="flex items-center gap-[18px] overflow-x-auto border-b ghost-border"
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
                  className={`-mb-px flex touch-target items-center justify-center whitespace-nowrap border-b-2 pb-2.5 pt-2 text-sm font-medium transition-colors ${
                    active
                      ? 'border-primary text-on-surface'
                      : 'border-transparent text-on-surface-variant hover:text-on-surface'
                  }`}
                >
                  {opt.label}
                </button>
              );
            })}
            <span data-testid="memory-fact-count" className="ml-auto whitespace-nowrap font-mono text-xs text-outline">
              {factCount(t, facts.length)}
            </span>
          </div>
        </div>
      </Section>

      {isLoading ? (
        <div className="flex justify-center py-8">
          <div className="pierre-spinner" />
        </div>
      ) : isError ? (
        <p className="text-sm text-error">
          {t('frag.failedLoadMemory')}{' '}
          {error instanceof Error ? error.message : String(error)}
        </p>
      ) : facts.length === 0 ? (
        // Two different absences. `facts` is the FILTERED list, so a type with
        // no matches told an athlete who has memory that they have none and
        // invited them to go earn some. The filtered case says what it means
        // and hands back the way out.
        <div data-testid={kindFilter === '' ? 'memory-empty' : 'memory-empty-filtered'} className="py-3">
          <p className="text-sm text-on-surface-variant">
            {kindFilter === '' ? t('shell.memoryEmpty') : t('shell.memoryEmptyFiltered')}
          </p>
          <p className="mt-1 text-xs text-outline">
            {kindFilter === '' ? t('shell.memoryEmptyHint') : t('shell.memoryEmptyFilteredHint')}
          </p>
          {kindFilter === '' ? null : (
            <div className="mt-3">
              <Button
                variant="tertiary"
                size="sm"
                onClick={() => setKindFilter('')}
                data-testid="memory-show-all-kinds"
              >
                {t('shell.memoryShowAllKinds')}
              </Button>
            </div>
          )}
        </div>
      ) : (
        <div className="space-y-8">
          {Array.from(groupedByKind.entries()).map(([kind, items]) => (
            <section key={kind}>
              <div className="flex items-center gap-2 border-b ghost-border-faint pb-2">
                <Badge variant={KIND_VARIANT[kind]}>{t(MEMORY_KIND_LABEL_KEY[kind])}</Badge>
                <span data-testid="memory-fact-count" className="text-xs text-on-surface-variant">
                  {factCount(t, items.length)}
                </span>
              </div>
              <ul>
                {items.map((fact) => (
                  <li key={fact.id} className="flex items-start justify-between gap-4 border-t ghost-border-faint py-3 first:border-t-0">
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
                      variant="tertiary"
                      size="sm"
                      onClick={() => setPendingForget(fact)}
                      disabled={forgetMutation.isPending}
                    >
                      {t('shell.memoryForget')}
                    </Button>
                  </li>
                ))}
              </ul>
            </section>
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
