// ABOUTME: Phase B Sprint C7 — admin tab for the coach followup triage queue
// ABOUTME: Lists pending followups tenant-wide, supports cancel to clear stale promises
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useMemo, useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { adminApi, type FollowupRow } from '../services/api/admin';
import { Card, Button, ConfirmDialog } from './ui';
import { useAuth } from '../hooks/useAuth';

const LIMIT_OPTIONS = [25, 50, 100, 200] as const;

function formatTimestamp(iso: string | null): string {
  if (!iso) {
    return '—';
  }
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function relativeFromNow(iso: string | null): string {
  if (!iso) {
    return 'no due date';
  }
  const then = Date.parse(iso);
  if (Number.isNaN(then)) {
    return iso;
  }
  const diffMs = then - Date.now();
  const absMin = Math.round(Math.abs(diffMs) / 60_000);
  if (absMin < 1) return 'now';
  if (absMin < 60) return diffMs >= 0 ? `in ${absMin}m` : `${absMin}m ago`;
  const absHours = Math.round(absMin / 60);
  if (absHours < 24) return diffMs >= 0 ? `in ${absHours}h` : `${absHours}h ago`;
  const absDays = Math.round(absHours / 24);
  return diffMs >= 0 ? `in ${absDays}d` : `${absDays}d ago`;
}

function isOverdue(iso: string | null): boolean {
  if (!iso) return false;
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return false;
  return then < Date.now();
}

export default function CoachFollowupsTab() {
  const { user } = useAuth();
  const tenantId = user?.tenant_id ?? '';
  const queryClient = useQueryClient();

  const [limit, setLimit] = useState<number>(100);
  const [coachFilter, setCoachFilter] = useState<string>('');
  const [userFilter, setUserFilter] = useState<string>('');
  const [pendingCancel, setPendingCancel] = useState<FollowupRow | null>(null);

  const queryKey = useMemo(
    () => ['admin', 'coach-followups', 'pending', tenantId, limit] as const,
    [tenantId, limit],
  );

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey,
    queryFn: () => adminApi.listPendingFollowups(tenantId, limit),
    enabled: Boolean(tenantId),
  });

  const cancelMutation = useMutation({
    mutationFn: (followupId: string) => adminApi.cancelFollowup(followupId, tenantId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin', 'coach-followups', 'pending'] });
      setPendingCancel(null);
    },
    onError: () => {
      setPendingCancel(null);
    },
  });

  const followups = useMemo(() => data?.followups ?? [], [data?.followups]);

  const filtered = useMemo(() => {
    const coachNeedle = coachFilter.trim();
    const userNeedle = userFilter.trim();
    return followups.filter((f) => {
      if (coachNeedle && !f.coach_id.includes(coachNeedle)) return false;
      if (userNeedle && !f.user_id.includes(userNeedle)) return false;
      return true;
    });
  }, [followups, coachFilter, userFilter]);

  const overdueCount = useMemo(
    () => filtered.filter((f) => isOverdue(f.due_at)).length,
    [filtered],
  );

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
              Coach followups
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Promised check-ins from coach personas waiting to be injected
              into the next system prompt. Cancel stale promises that should
              never reach the coach.
            </p>
          </div>
          <Button onClick={() => refetch()} variant="secondary" disabled={isFetching}>
            {isFetching ? 'Refreshing…' : 'Refresh'}
          </Button>
        </div>

        <div className="mt-6 grid grid-cols-1 gap-3 md:grid-cols-3">
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Pending
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-white">
              {filtered.length}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Overdue
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-white">
              {overdueCount}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Total fetched
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-white">
              {data?.total ?? 0}
            </div>
          </div>
        </div>
      </Card>

      <Card className="p-6">
        <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
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
            Failed to load followups:{' '}
            {error instanceof Error ? error.message : String(error)}
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-12 text-center">
            <p className="text-gray-500 dark:text-gray-400">No pending followups.</p>
            <p className="mt-1 text-xs text-gray-400 dark:text-gray-500">
              Coach personas add followups via tool calls when they promise a
              future check-in. Nothing pending means every promise has been
              delivered or cancelled.
            </p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
              <thead className="bg-gray-50 dark:bg-gray-800">
                <tr>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Content
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Due
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Coach
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    User
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Created
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Action
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200 bg-white dark:divide-gray-700 dark:bg-gray-900">
                {filtered.map((f) => {
                  const overdue = isOverdue(f.due_at);
                  return (
                    <tr key={f.id}>
                      <td className="max-w-md px-4 py-3 text-sm text-gray-900 dark:text-gray-100">
                        {f.content}
                      </td>
                      <td className="px-4 py-3 text-sm">
                        <span
                          className={
                            overdue
                              ? 'font-medium text-red-600 dark:text-red-400'
                              : 'text-gray-600 dark:text-gray-300'
                          }
                        >
                          {relativeFromNow(f.due_at)}
                        </span>
                        <div className="text-xs text-gray-400 dark:text-gray-500">
                          {formatTimestamp(f.due_at)}
                        </div>
                      </td>
                      <td className="px-4 py-3 font-mono text-xs text-gray-600 dark:text-gray-300">
                        {f.coach_id}
                      </td>
                      <td className="px-4 py-3 font-mono text-xs text-gray-600 dark:text-gray-300">
                        {f.user_id}
                      </td>
                      <td className="px-4 py-3 text-xs text-gray-500 dark:text-gray-400">
                        {formatTimestamp(f.created_at)}
                      </td>
                      <td className="px-4 py-3 text-right">
                        <Button
                          variant="secondary"
                          onClick={() => setPendingCancel(f)}
                          disabled={cancelMutation.isPending}
                        >
                          Cancel
                        </Button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      {pendingCancel ? (
        <ConfirmDialog
          isOpen
          title="Cancel this followup?"
          message={`The coach will never see "${pendingCancel.content}". This cannot be undone.`}
          confirmLabel="Cancel followup"
          cancelLabel="Keep"
          variant="danger"
          isLoading={cancelMutation.isPending}
          onConfirm={() => cancelMutation.mutate(pendingCancel.id)}
          onClose={() => setPendingCancel(null)}
        />
      ) : null}
    </div>
  );
}
