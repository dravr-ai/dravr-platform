// ABOUTME: Phase D Sprint C14 — admin tab listing per-agent claim-verification content grades
// ABOUTME: Worst grades first so admins can review agents at the bottom of the leaderboard
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import {
  adminApi,
  type AgentGrade,
  type AgentGradingSummary,
  type LetterGrade,
} from '../services/api/admin';
import { Card, Button, Badge } from './ui';
import { useAdminScope } from '../hooks/useAdminScope';
import AdminScopeBar from './AdminScopeBar';
import { describeAdminError } from '../utils/adminLocale';

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
  const { requested } = useAdminScope();

  const { data, isLoading, isError, error, refetch, isFetching } =
    useQuery<AgentGradingSummary>({
      queryKey: ['admin', 'coach-grading', requested] as const,
      queryFn: () => adminApi.getCoachGradingSummary(requested, 500),
      refetchInterval: 120_000,
    });

  return (
    <div className="space-y-4">
      <AdminScopeBar scope={data?.scope} />
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-on-surface">
              Agent content grades
            </h2>
            <p className="mt-1 text-sm text-on-surface-variant">
              Per-agent scores from claim verdicts: supported claims raise a score,
              contradicted claims lower it. Grades count verdicts made after{' '}
              <span className="font-medium" data-testid="judge-era-after">{data?.judge_era_after ?? '—'}</span>
              , when the LLM judge began settling inconclusive claims; before that a corpus miss read as
              unsupported. Worst-graded agents appear first.
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

        <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-4">
          <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
            <div className="text-xs text-on-surface-variant">
              Judge-era verdicts
            </div>
            <div className="mt-1 text-xl font-semibold text-on-surface" data-testid="judge-era-verdicts">
              {(data?.verdicts_scanned ?? 0) - (data?.verdicts_before_judge_era ?? 0)}
            </div>
            <div className="mt-1 text-xs text-on-surface-variant" data-testid="older-verdicts">
              {data?.verdicts_before_judge_era ?? 0} older not counted
            </div>
          </div>
          <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
            <div className="text-xs text-on-surface-variant">
              Agents graded
            </div>
            <div className="mt-1 text-xl font-semibold text-on-surface">
              {data?.grades.length ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
            <div className="text-xs text-on-surface-variant">
              Low grades (D / F)
            </div>
            <div className="mt-1 text-xl font-semibold text-on-surface">
              {data?.grades.filter((g) => g.grade === 'D' || g.grade === 'F').length ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
            <div className="text-xs text-on-surface-variant">
              No agent on record
            </div>
            <div className="mt-1 text-xl font-semibold text-on-surface" data-testid="unattributed-verdicts">
              {data?.unattributed_verdicts ?? 0}
            </div>
            <div className="mt-1 text-xs text-on-surface-variant">
              Judge-era verdicts in no grade
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
          <div className="p-6 text-sm text-error">
            Failed to load agent grading summary:{' '}
            {describeAdminError(error)}
          </div>
        ) : !data || data.grades.length === 0 ? (
          <div className="p-12 text-center">
            <p className="text-on-surface-variant">
              No agent grades available yet.
            </p>
            <p className="mt-1 text-xs text-outline">
              Agents need at least 3 scored claim verdicts for a letter grade;
              fewer read as provisional.
            </p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-outline-variant">
              <thead className="bg-surface-container">
                <tr>
                  <th className="px-4 py-2 text-left text-xs font-medium text-on-surface-variant">
                    Grade
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-on-surface-variant">
                    Agent
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium text-on-surface-variant">
                    Score
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium text-on-surface-variant">
                    Total
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium text-on-surface-variant">
                    Supported
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium text-on-surface-variant">
                    Unsupported
                  </th>
                  <th className="px-4 py-2 text-right text-xs font-medium text-on-surface-variant">
                    Contradicted
                  </th>
                  <th
                    className="px-4 py-2 text-right text-xs font-medium text-on-surface-variant"
                    title="Left out of the score: rhetorical, unverifiable, unsupported training advice and false positives"
                  >
                    Not scored
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-outline-variant">
                {data.grades.map((g: AgentGrade) => (
                  <tr key={g.agent_id}>
                    <td className="px-4 py-3">
                      <Badge variant={GRADE_VARIANT[g.grade]}>{g.grade}</Badge>
                    </td>
                    <td className="px-4 py-3 text-sm text-on-surface">
                      <div className="font-medium">{g.agent_name ?? 'Agent not found'}</div>
                      <div className="font-mono text-xs text-on-surface-variant">{g.agent_id}</div>
                    </td>
                    <td className="px-4 py-3 text-right text-sm font-semibold text-on-surface">
                      {formatScore(g.score)}
                    </td>
                    <td className="px-4 py-3 text-right text-xs text-on-surface-variant">
                      {g.total_verdicts}
                    </td>
                    <td className="px-4 py-3 text-right text-xs text-on-surface-variant">
                      {g.supported}
                    </td>
                    <td className="px-4 py-3 text-right text-xs text-on-surface-variant">
                      {g.unsupported}
                    </td>
                    <td className="px-4 py-3 text-right text-xs text-on-surface-variant">
                      {g.contradicted}
                    </td>
                    <td
                      className="px-4 py-3 text-right text-xs text-on-surface-variant"
                      data-testid="not-scored"
                      title={`${g.rhetorical} rhetorical, ${g.unverifiable} unverifiable, ${g.unsupported_prescription} unsupported training advice, ${g.false_positives} false positives`}
                    >
                      {g.rhetorical + g.unverifiable + g.unsupported_prescription + g.false_positives}
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
