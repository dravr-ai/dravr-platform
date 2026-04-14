// ABOUTME: Tier 5.5 admin tab — triage flagged claim verdicts from the bullshit detector
// ABOUTME: Lists claim_verdicts with status/category/coach filters, drill-down drawer for details
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { adminApi, type ClaimVerdictRow } from '../services/api/admin';
import { Card, Button, Badge } from './ui';
import ClaimVerdictDrawer from './ClaimVerdictDrawer';
import { useAuth } from '../hooks/useAuth';

const STATUS_OPTIONS = [
  { value: '', label: 'All statuses' },
  { value: 'supported', label: 'Supported' },
  { value: 'unsupported', label: 'Unsupported' },
  { value: 'contradicted', label: 'Contradicted' },
  { value: 'rhetorical', label: 'Rhetorical' },
  { value: 'unverifiable', label: 'Unverifiable' },
] as const;

const CATEGORY_OPTIONS = [
  { value: '', label: 'All categories' },
  { value: 'physiological', label: 'Physiological' },
  { value: 'training_prescription', label: 'Training prescription' },
  { value: 'nutrition', label: 'Nutrition' },
  { value: 'recovery', label: 'Recovery' },
  { value: 'supplement', label: 'Supplement' },
  { value: 'injury_rehab', label: 'Injury rehab' },
] as const;

const LIMIT_OPTIONS = [25, 50, 100, 200] as const;

type StatusKey = ClaimVerdictRow['status'];
type StrengthKey = ClaimVerdictRow['evidence_strength'];
type BadgeVariant = 'success' | 'warning' | 'error' | 'info' | 'secondary';

const STATUS_VARIANT: Record<StatusKey, BadgeVariant> = {
  supported: 'success',
  unsupported: 'warning',
  contradicted: 'error',
  rhetorical: 'info',
  unverifiable: 'secondary',
};

const STRENGTH_VARIANT: Record<StrengthKey, BadgeVariant> = {
  strong: 'success',
  mixed: 'info',
  weak: 'warning',
  none: 'secondary',
};

function formatTimestamp(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function humanizeCategory(cat: string): string {
  return cat.replace(/_/g, ' ').replace(/\b\w/g, (c: string) => c.toUpperCase());
}

export default function ClaimVerdictsTab() {
  const { user } = useAuth();
  const tenantId = user?.tenant_id ?? '';

  const [statusFilter, setStatusFilter] = useState<string>('');
  const [categoryFilter, setCategoryFilter] = useState<string>('');
  const [coachFilter, setCoachFilter] = useState<string>('');
  const [limit, setLimit] = useState<number>(50);
  const [selectedVerdict, setSelectedVerdict] = useState<ClaimVerdictRow | null>(null);

  const queryKey = useMemo(
    () => ['admin', 'claim-verdicts', tenantId, statusFilter, categoryFilter, coachFilter, limit] as const,
    [tenantId, statusFilter, categoryFilter, coachFilter, limit],
  );

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey,
    queryFn: () =>
      adminApi.listClaimVerdicts({
        tenant_id: tenantId,
        status: statusFilter || undefined,
        category: categoryFilter || undefined,
        coach_id: coachFilter || undefined,
        limit,
      }),
    enabled: Boolean(tenantId),
  });

  const verdicts = useMemo(() => data?.verdicts ?? [], [data?.verdicts]);
  const total = data?.total ?? 0;

  const statusCounts = useMemo(() => {
    const counts: Record<StatusKey, number> = {
      supported: 0,
      unsupported: 0,
      contradicted: 0,
      rhetorical: 0,
      unverifiable: 0,
    };
    for (const v of verdicts) {
      counts[v.status] += 1;
    }
    return counts;
  }, [verdicts]);

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
              Tier 5.5 — Claim verdicts
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Every claim emitted by a coach persona that runs through the bullshit
              detector ends up here. Filter, drill in, and course-correct coaches
              that ship unsupported or contradicted claims.
            </p>
          </div>
          <Button onClick={() => refetch()} variant="secondary">
            Refresh
          </Button>
        </div>

        <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-5">
          {(Object.entries(statusCounts) as [StatusKey, number][]).map(([key, count]) => (
            <div
              key={key}
              className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800"
            >
              <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
                {key}
              </div>
              <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-white">
                {count}
              </div>
            </div>
          ))}
        </div>
      </Card>

      <Card className="p-6">
        <div className="grid grid-cols-1 gap-3 md:grid-cols-4">
          <label className="flex flex-col gap-1 text-sm">
            <span className="font-medium text-gray-700 dark:text-gray-300">Status</span>
            <select
              value={statusFilter}
              onChange={(e) => setStatusFilter(e.target.value)}
              className="rounded border border-gray-300 bg-white px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-white"
            >
              {STATUS_OPTIONS.map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {opt.label}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-sm">
            <span className="font-medium text-gray-700 dark:text-gray-300">Category</span>
            <select
              value={categoryFilter}
              onChange={(e) => setCategoryFilter(e.target.value)}
              className="rounded border border-gray-300 bg-white px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-white"
            >
              {CATEGORY_OPTIONS.map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {opt.label}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-sm">
            <span className="font-medium text-gray-700 dark:text-gray-300">Coach ID</span>
            <input
              type="text"
              value={coachFilter}
              onChange={(e) => setCoachFilter(e.target.value.trim())}
              placeholder="filter by coach id"
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
            Failed to load verdicts: {error instanceof Error ? error.message : String(error)}
          </div>
        ) : verdicts.length === 0 ? (
          <div className="p-12 text-center">
            <p className="text-gray-500 dark:text-gray-400">
              No claim verdicts matching these filters.
            </p>
            <p className="mt-1 text-xs text-gray-400 dark:text-gray-500">
              Verdicts are written when a coach reply passes through the Tier 5.5
              detector pipeline with claims that need verification.
            </p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
              <thead className="bg-gray-50 dark:bg-gray-800">
                <tr>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Claim
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Status
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Strength
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Category
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    Layer
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
                    When
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200 bg-white dark:divide-gray-700 dark:bg-gray-900">
                {verdicts.map((v) => (
                  <tr
                    key={v.id}
                    className={clsx(
                      'cursor-pointer hover:bg-gray-50 dark:hover:bg-gray-800',
                      selectedVerdict?.id === v.id && 'bg-pierre-violet/10',
                    )}
                    onClick={() => setSelectedVerdict(v)}
                  >
                    <td className="max-w-md px-4 py-3 text-sm text-gray-900 dark:text-gray-100">
                      <span className="line-clamp-2">{v.claim_text}</span>
                    </td>
                    <td className="px-4 py-3">
                      <Badge variant={STATUS_VARIANT[v.status]}>{v.status}</Badge>
                    </td>
                    <td className="px-4 py-3">
                      <Badge variant={STRENGTH_VARIANT[v.evidence_strength]}>
                        {v.evidence_strength}
                      </Badge>
                    </td>
                    <td className="px-4 py-3 text-sm text-gray-600 dark:text-gray-400">
                      {humanizeCategory(v.category)}
                    </td>
                    <td className="px-4 py-3 text-xs text-gray-500 dark:text-gray-500">
                      {v.layer_fired}
                    </td>
                    <td className="px-4 py-3 text-xs text-gray-500 dark:text-gray-500">
                      {formatTimestamp(v.created_at)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {total > 0 ? (
          <div className="border-t border-gray-200 bg-gray-50 px-4 py-2 text-xs text-gray-500 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-400">
            Showing {total} verdict{total === 1 ? '' : 's'}
          </div>
        ) : null}
      </Card>

      {selectedVerdict ? (
        <ClaimVerdictDrawer
          verdict={selectedVerdict}
          onClose={() => setSelectedVerdict(null)}
        />
      ) : null}
    </div>
  );
}
