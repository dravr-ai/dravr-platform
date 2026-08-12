// ABOUTME: Claim-verification admin tab — triage flagged claim verdicts from the bullshit detector
// ABOUTME: Lists claim_verdicts with status/category/coach filters, drill-down drawer for details
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { adminApi, type ClaimVerdictRow } from '../services/api/admin';
import { Card, Button, Badge, Select , Input } from './ui';
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
        <p className="text-sm text-on-surface-variant">
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
            <h2 className="text-xl font-semibold text-on-surface">
              Claim verdicts
            </h2>
            <p className="mt-1 text-sm text-on-surface-variant">
              Every claim a coach makes is evaluated by the claim verifier and
              recorded here. Filter, drill into the source message, and
              course-correct coaches that ship unsupported or contradicted
              claims.
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
              className="rounded-lg border border-outline-variant bg-surface-container p-3"
            >
              <div className="text-xs uppercase tracking-wide text-on-surface-variant">
                {key}
              </div>
              <div className="mt-1 text-2xl font-semibold text-on-surface">
                {count}
              </div>
            </div>
          ))}
        </div>
      </Card>

      <Card className="p-6">
        <div className="grid grid-cols-1 gap-3 md:grid-cols-4">
          <Select
            label="Status"
            value={statusFilter}
            onChange={(e) => setStatusFilter(e.target.value)}
            options={STATUS_OPTIONS.map((opt) => ({ value: opt.value, label: opt.label }))}
          />
          <Select
            label="Category"
            value={categoryFilter}
            onChange={(e) => setCategoryFilter(e.target.value)}
            options={CATEGORY_OPTIONS.map((opt) => ({ value: opt.value, label: opt.label }))}
          />
          <Input label="Coach ID" type="text" value={coachFilter} onChange={(e) => setCoachFilter(e.target.value.trim())} placeholder="filter by coach id" />
          <Select
            label="Rows"
            value={limit}
            onChange={(e) => setLimit(Number(e.target.value))}
            options={LIMIT_OPTIONS.map((n) => ({ value: String(n), label: String(n) }))}
          />
        </div>
      </Card>

      <Card className="overflow-hidden">
        {isLoading ? (
          <div className="flex justify-center p-12">
            <div className="pierre-spinner" />
          </div>
        ) : isError ? (
          <div className="p-6 text-sm text-error">
            Failed to load verdicts: {error instanceof Error ? error.message : String(error)}
          </div>
        ) : verdicts.length === 0 ? (
          <div className="p-12 text-center">
            <p className="text-on-surface-variant">
              No claim verdicts matching these filters.
            </p>
            <p className="mt-1 text-xs text-outline">
              Verdicts are written when a coach reply passes through the claim
              verifier and produces claims that need evidence.
            </p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-outline-variant">
              <thead className="bg-surface-container">
                <tr>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-on-surface-variant">
                    Claim
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-on-surface-variant">
                    Status
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-on-surface-variant">
                    Strength
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-on-surface-variant">
                    Category
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-on-surface-variant">
                    Layer
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium uppercase tracking-wide text-on-surface-variant">
                    When
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-outline-variant bg-white">
                {verdicts.map((v) => (
                  <tr
                    key={v.id}
                    className={clsx(
                      'cursor-pointer hover:bg-surface-container dark:hover:bg-surface-container',
                      selectedVerdict?.id === v.id && 'bg-primary/10',
                    )}
                    onClick={() => setSelectedVerdict(v)}
                  >
                    <td className="max-w-md px-4 py-3 text-sm text-on-surface">
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
                    <td className="px-4 py-3 text-sm text-on-surface-variant">
                      {humanizeCategory(v.category)}
                    </td>
                    <td className="px-4 py-3 text-xs text-on-surface-variant">
                      {v.layer_fired}
                    </td>
                    <td className="px-4 py-3 text-xs text-on-surface-variant">
                      {formatTimestamp(v.created_at)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {total > 0 ? (
          <div className="border-t border-outline-variant bg-surface-container px-4 py-2 text-xs text-on-surface-variant">
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
