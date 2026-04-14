// ABOUTME: Phase B Sprint C8 — admin compliance audit log for coach-authored notes
// ABOUTME: Flat tenant-wide feed of every coach note with coach/user filters and scope badge
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { adminApi, type CoachNoteAuditRow } from '../services/api/admin';
import { Card, Button, Badge } from './ui';
import { useAuth } from '../hooks/useAuth';

const LIMIT_OPTIONS = [50, 100, 200, 500] as const;

type ScopeKey = CoachNoteAuditRow['scope'];
type BadgeVariant = 'success' | 'warning' | 'error' | 'info' | 'secondary';

const SCOPE_VARIANT: Record<ScopeKey, BadgeVariant> = {
  conversation: 'info',
  user: 'success',
  tenant: 'warning',
};

function formatTimestamp(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function humanizeScope(scope: string): string {
  return scope.charAt(0).toUpperCase() + scope.slice(1);
}

export default function CoachNotesAuditTab() {
  const { user } = useAuth();
  const tenantId = user?.tenant_id ?? '';

  const [limit, setLimit] = useState<number>(100);
  const [coachFilter, setCoachFilter] = useState<string>('');
  const [userFilter, setUserFilter] = useState<string>('');
  const [scopeFilter, setScopeFilter] = useState<'' | ScopeKey>('');
  const [searchText, setSearchText] = useState<string>('');

  const queryKey = useMemo(
    () => ['admin', 'coach-notes', 'audit', tenantId, limit] as const,
    [tenantId, limit],
  );

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey,
    queryFn: () => adminApi.listCoachNoteAudit(tenantId, limit),
    enabled: Boolean(tenantId),
  });

  const notes = useMemo(() => data?.notes ?? [], [data?.notes]);

  const filtered = useMemo(() => {
    const coachNeedle = coachFilter.trim();
    const userNeedle = userFilter.trim();
    const textNeedle = searchText.trim().toLowerCase();
    return notes.filter((n) => {
      if (coachNeedle && !n.coach_id.includes(coachNeedle)) return false;
      if (userNeedle && !n.user_id.includes(userNeedle)) return false;
      if (scopeFilter && n.scope !== scopeFilter) return false;
      if (textNeedle && !n.content.toLowerCase().includes(textNeedle)) return false;
      return true;
    });
  }, [notes, coachFilter, userFilter, scopeFilter, searchText]);

  const scopeCounts = useMemo(() => {
    const counts: Record<ScopeKey, number> = { conversation: 0, user: 0, tenant: 0 };
    for (const n of filtered) {
      counts[n.scope] += 1;
    }
    return counts;
  }, [filtered]);

  if (!tenantId) {
    return (
      <Card className="p-6">
        <p className="text-sm text-gray-500 dark:text-gray-400">
          Tenant id not available on your session. Reload the page and try again.
        </p>
      </Card>
    );
  }

  return (
    <div className="space-y-4">
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-gray-900 dark:text-white">
              Coach notes — compliance audit
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Flat tenant-wide audit of every note a coach persona wrote about
              a user. Compliance reviewers can search, filter, and export
              without stitching together (user, coach) pairs.
            </p>
          </div>
          <Button onClick={() => refetch()} variant="secondary" disabled={isFetching}>
            {isFetching ? 'Refreshing…' : 'Refresh'}
          </Button>
        </div>

        <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-4">
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Notes shown
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-white">
              {filtered.length}
            </div>
          </div>
          {(Object.entries(scopeCounts) as [ScopeKey, number][]).map(([key, count]) => (
            <div
              key={key}
              className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800"
            >
              <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
                {humanizeScope(key)}
              </div>
              <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-white">
                {count}
              </div>
            </div>
          ))}
        </div>
      </Card>

      <Card className="p-6">
        <div className="grid grid-cols-1 gap-3 md:grid-cols-5">
          <label className="flex flex-col gap-1 text-sm md:col-span-2">
            <span className="font-medium text-gray-700 dark:text-gray-300">Search content</span>
            <input
              type="text"
              value={searchText}
              onChange={(e) => setSearchText(e.target.value)}
              placeholder="substring search in note body"
              className="rounded border border-gray-300 bg-white px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-white"
            />
          </label>
          <label className="flex flex-col gap-1 text-sm">
            <span className="font-medium text-gray-700 dark:text-gray-300">Coach ID</span>
            <input
              type="text"
              value={coachFilter}
              onChange={(e) => setCoachFilter(e.target.value)}
              placeholder="filter by coach id"
              className="rounded border border-gray-300 bg-white px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-white"
            />
          </label>
          <label className="flex flex-col gap-1 text-sm">
            <span className="font-medium text-gray-700 dark:text-gray-300">User ID</span>
            <input
              type="text"
              value={userFilter}
              onChange={(e) => setUserFilter(e.target.value)}
              placeholder="filter by user id"
              className="rounded border border-gray-300 bg-white px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-white"
            />
          </label>
          <label className="flex flex-col gap-1 text-sm">
            <span className="font-medium text-gray-700 dark:text-gray-300">Scope</span>
            <select
              value={scopeFilter}
              onChange={(e) => setScopeFilter(e.target.value as '' | ScopeKey)}
              className="rounded border border-gray-300 bg-white px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-white"
            >
              <option value="">All scopes</option>
              <option value="conversation">Conversation</option>
              <option value="user">User</option>
              <option value="tenant">Tenant</option>
            </select>
          </label>
          <label className="flex flex-col gap-1 text-sm">
            <span className="font-medium text-gray-700 dark:text-gray-300">Rows</span>
            <select
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
              className="rounded border border-gray-300 bg-white px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-white"
            >
              {LIMIT_OPTIONS.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
        </div>
      </Card>

      <Card className="overflow-hidden">
        {isLoading ? (
          <div className="flex justify-center p-12">
            <div className="pierre-spinner" />
          </div>
        ) : isError ? (
          <div className="p-6 text-sm text-red-600 dark:text-red-400">
            Failed to load coach notes audit:{' '}
            {error instanceof Error ? error.message : String(error)}
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-12 text-center">
            <p className="text-gray-500 dark:text-gray-400">
              No coach notes match these filters.
            </p>
            <p className="mt-1 text-xs text-gray-400 dark:text-gray-500">
              Coach personas write notes about a user via the
              <code className="mx-1 rounded bg-gray-100 px-1 dark:bg-gray-800">
                memory.write_note
              </code>
              tool. Nothing listed means no notes have been authored in this tenant yet.
            </p>
          </div>
        ) : (
          <ul className="divide-y divide-gray-200 dark:divide-gray-700">
            {filtered.map((note) => (
              <li key={note.id} className="px-6 py-4">
                <div className="flex items-start justify-between gap-4">
                  <div className="flex flex-wrap items-center gap-2 text-xs text-gray-500 dark:text-gray-400">
                    <Badge variant={SCOPE_VARIANT[note.scope]}>{humanizeScope(note.scope)}</Badge>
                    <span className="font-mono">coach {note.coach_id}</span>
                    <span className="text-gray-400 dark:text-gray-500">·</span>
                    <span className="font-mono">user {note.user_id}</span>
                    {note.conversation_id ? (
                      <>
                        <span className="text-gray-400 dark:text-gray-500">·</span>
                        <span className="font-mono">conv {note.conversation_id}</span>
                      </>
                    ) : null}
                  </div>
                  <span className="shrink-0 text-xs text-gray-400 dark:text-gray-500">
                    {formatTimestamp(note.created_at)}
                  </span>
                </div>
                <p className="mt-2 whitespace-pre-wrap text-sm text-gray-900 dark:text-gray-100">
                  {note.content}
                </p>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </div>
  );
}
