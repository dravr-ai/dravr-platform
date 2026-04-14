// ABOUTME: Phase B Sprint C5 — user-facing memory panel with GDPR Forget per-fact
// ABOUTME: Lists pierre-memory user_facts grouped by kind, lets users forget individual rows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useMemo, useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type { MemoryFactRow } from '@pierre/api-client';
import { userApi } from '../../services/api';
import { Card, Button, Badge, ConfirmDialog } from '../ui';

const MEMORY_FACTS_QUERY_KEY = ['memory', 'facts'] as const;

const KIND_OPTIONS: { value: MemoryFactRow['kind'] | ''; label: string }[] = [
  { value: '', label: 'All kinds' },
  { value: 'preference', label: 'Preference' },
  { value: 'physiology', label: 'Physiology' },
  { value: 'injury', label: 'Injury' },
  { value: 'goal', label: 'Goal' },
  { value: 'schedule', label: 'Schedule' },
  { value: 'equipment', label: 'Equipment' },
  { value: 'other', label: 'Other' },
];

const KIND_VARIANT: Record<MemoryFactRow['kind'], 'success' | 'info' | 'warning' | 'error' | 'secondary'> = {
  preference: 'info',
  physiology: 'success',
  injury: 'warning',
  goal: 'info',
  schedule: 'secondary',
  equipment: 'secondary',
  other: 'secondary',
};

function humanizeKind(kind: string): string {
  return kind.charAt(0).toUpperCase() + kind.slice(1);
}

function formatTimestamp(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

export default function MemoryPanel() {
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
            <h2 className="text-xl font-semibold text-gray-900 dark:text-on-surface">
              What the coach remembers about you
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Facts the platform extracted from your conversations to give the
              coach memory across sessions. You can forget any individual fact
              and the coach will stop using it on the next turn.
            </p>
          </div>
          <Button variant="secondary" onClick={() => refetch()}>
            Refresh
          </Button>
        </div>

        <div className="mt-4 flex flex-wrap items-center gap-3">
          <label className="text-sm font-medium text-gray-700 dark:text-gray-300">
            Filter by kind
          </label>
          <select
            value={kindFilter}
            onChange={(e) =>
              setKindFilter((e.target.value || '') as MemoryFactRow['kind'] | '')
            }
            className="rounded border border-gray-300 bg-white px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-on-surface"
          >
            {KIND_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
          <span className="text-xs text-gray-500 dark:text-gray-400">
            {facts.length} fact{facts.length === 1 ? '' : 's'}
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
          <p className="text-sm text-red-600 dark:text-red-400">
            Failed to load memory facts:{' '}
            {error instanceof Error ? error.message : String(error)}
          </p>
        </Card>
      ) : facts.length === 0 ? (
        <Card className="p-12 text-center">
          <p className="text-gray-500 dark:text-gray-400">
            No facts stored yet.
          </p>
          <p className="mt-2 text-xs text-gray-400 dark:text-gray-500">
            Talk to a coach for a few turns and the platform will start
            building your memory profile.
          </p>
        </Card>
      ) : (
        <div className="space-y-4">
          {Array.from(groupedByKind.entries()).map(([kind, items]) => (
            <Card key={kind} className="overflow-hidden">
              <div className="border-b border-gray-200 bg-gray-50 px-4 py-2 dark:border-gray-700 dark:bg-gray-800">
                <div className="flex items-center gap-2">
                  <Badge variant={KIND_VARIANT[kind]}>{humanizeKind(kind)}</Badge>
                  <span className="text-xs text-gray-500 dark:text-gray-400">
                    {items.length} fact{items.length === 1 ? '' : 's'}
                  </span>
                </div>
              </div>
              <ul className="divide-y divide-gray-200 dark:divide-gray-700">
                {items.map((fact) => (
                  <li key={fact.id} className="flex items-start justify-between gap-4 px-4 py-3">
                    <div className="min-w-0 flex-1">
                      <p className="text-sm text-gray-900 dark:text-gray-100">
                        <span className="font-medium">{fact.subject}</span>{' '}
                        <span className="text-gray-600 dark:text-gray-400">{fact.predicate}</span>{' '}
                        <span className="font-medium">{fact.object}</span>
                      </p>
                      <p className="mt-1 text-xs text-gray-500 dark:text-gray-500">
                        Confidence {(fact.confidence * 100).toFixed(0)}% ·{' '}
                        Updated {formatTimestamp(fact.updated_at)}
                        {fact.coach_id ? ` · Coach ${fact.coach_id}` : ''}
                      </p>
                    </div>
                    <Button
                      variant="secondary"
                      onClick={() => setPendingForget(fact)}
                      disabled={forgetMutation.isPending}
                    >
                      Forget
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
          title="Forget this fact?"
          message={`The coach will stop using "${pendingForget.subject} ${pendingForget.predicate} ${pendingForget.object}" on the next turn. This cannot be undone.`}
          confirmLabel="Forget"
          cancelLabel="Cancel"
          variant="danger"
          isLoading={forgetMutation.isPending}
          onConfirm={() => forgetMutation.mutate(pendingForget.id)}
          onClose={() => setPendingForget(null)}
        />
      ) : null}
    </div>
  );
}
