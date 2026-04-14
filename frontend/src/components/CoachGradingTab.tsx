// ABOUTME: Phase D Sprint C14 — admin tab listing per-coach Tier 5.5 content grades
// ABOUTME: Worst grades first so admins can review coaches at the bottom of the leaderboard
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import {
  adminApi,
  type CoachGrade,
  type CoachGradingSummary,
  type LetterGrade,
} from '../services/api/admin';
import { Card, Button, Badge } from './ui';
import { useAuth } from '../hooks/useAuth';

type BadgeVariant = 'success' | 'warning' | 'error' | 'info' | 'secondary';

const GRADE_VARIANT: Record<LetterGrade, BadgeVariant> = {
  A: 'success',
  B: 'success',
  C: 'info',
  D: 'warning',
  F: 'error',
  PROVISIONAL: 'secondary',
};

function formatScore(score: number): string {
  return `${(score * 100).toFixed(0)}%`;
}

export default function CoachGradingTab() {
  const { user } = useAuth();
  const tenantId = user?.tenant_id ?? '';

  const { data, isLoading, isError, error, refetch, isFetching } =
    useQuery<CoachGradingSummary>({
      queryKey: ['admin', 'coach-grading', tenantId] as const,
      queryFn: () => adminApi.getCoachGradingSummary(tenantId, 500),
      enabled: Boolean(tenantId),
      refetchInterval: 120_000,
    });

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
            <h2 className="text-xl font-semibold text-gray-900 dark:text-on-surface">
              Coach content grades
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Per-coach quality scores derived from Tier 5.5 verdict
              history. Higher scores rank higher in the store. Worst-graded
              coaches appear first so admins can review or unpublish them.
            </p>
          </div>
          <Button
            onClick={() => refetch()}
            variant="secondary"
            disabled={isFetching}
          >
            {isFetching ? 'Refreshing…' : 'Refresh'}
          </Button>
        </div>

        <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-3">
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Verdicts scanned
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-on-surface">
              {data?.verdicts_scanned ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Coaches graded
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-on-surface">
              {data?.grades.length ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Failing (D / F)
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-on-surface">
              {data?.grades.filter((g) => g.grade === 'D' || g.grade === 'F').length ?? 0}
            </div>
          </div>
        </div>
      </Card>

      <Card className="overflow-hidden">
        {isLoading ? (
          <div className="flex justify-center p-12">
            <div className="pierre-spinner" />
          </div>
        ) : isError ? (
          <div className="p-6 text-sm text-red-600 dark:text-red-400">
            Failed to load coach grading summary:{' '}
            {error instanceof Error ? error.message : String(error)}
          </div>
        ) : !data || data.grades.length === 0 ? (
          <div className="p-12 text-center">
            <p className="text-gray-500 dark:text-gray-400">
              No coach grades available yet.
            </p>
            <p className="mt-1 text-xs text-gray-400 dark:text-gray-500">
              Coaches need at least a few Tier 5.5 verdicts before they can
              be graded.
            </p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
              <thead className="bg-gray-50 dark:bg-gray-800">
                <tr>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Grade
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Coach
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Score
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Total
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Supported
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Unsupported
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Contradicted
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200 bg-white dark:divide-gray-700 dark:bg-gray-900">
                {data.grades.map((g: CoachGrade) => (
                  <tr key={g.coach_id}>
                    <td className="px-4 py-3">
                      <Badge variant={GRADE_VARIANT[g.grade]}>{g.grade}</Badge>
                    </td>
                    <td className="px-4 py-3 font-mono text-xs text-gray-900 dark:text-gray-100">
                      {g.coach_id}
                    </td>
                    <td className="px-4 py-3 text-right text-sm font-semibold text-gray-900 dark:text-on-surface">
                      {formatScore(g.score)}
                    </td>
                    <td className="px-4 py-3 text-right text-xs text-gray-600 dark:text-gray-300">
                      {g.total_verdicts}
                    </td>
                    <td className="px-4 py-3 text-right text-xs text-gray-600 dark:text-gray-300">
                      {g.supported}
                    </td>
                    <td className="px-4 py-3 text-right text-xs text-gray-600 dark:text-gray-300">
                      {g.unsupported}
                    </td>
                    <td className="px-4 py-3 text-right text-xs text-gray-600 dark:text-gray-300">
                      {g.contradicted}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>
    </div>
  );
}
