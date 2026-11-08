// ABOUTME: Phase D Sprint C13 — admin myth-busting tab summarizing unsupported claims
// ABOUTME: Recent claims athletes were warned about with their dates, top offending agents, top categories from claim_verdicts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import { adminApi, type MythBustingSummary } from '../services/api/admin';
import { Card, Button } from './ui';
import { useAdminScope } from '../hooks/useAdminScope';
import AdminScopeBar from './AdminScopeBar';
import { ADMIN_LOCALE, describeAdminError } from '../utils/adminLocale';

function formatTimestamp(iso: string | null): string {
  if (!iso) return '—';
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

/// The same window Agent Grades reads, so the two screens count the same rows.
const VERDICTS_READ = 500;

function formatDate(iso: string | null): string {
  if (!iso) return '—';
  return new Date(iso).toLocaleDateString(ADMIN_LOCALE);
}

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? '' : 's'}`;
}

function humanizeCategory(cat: string): string {
  return cat.replace(/_/g, ' ').replace(/\b\w/g, (c: string) => c.toUpperCase());
}

export default function MythBustingTab() {
  const { requested } = useAdminScope();

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery<MythBustingSummary>({
    queryKey: ['admin', 'myth-busting', requested] as const,
    queryFn: () => adminApi.getMythBustingSummary(requested, VERDICTS_READ),
    refetchInterval: 120_000,
  });


  return (
    <div className="space-y-4">
      <AdminScopeBar scope={data?.scope} />
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-on-surface">
              Myth-busting — recent claims athletes were warned about
            </h2>
            <p className="mt-1 text-sm text-on-surface-variant">
              The latest <span className="font-medium">{data?.verdicts_scanned ?? '—'}</span>{' '}
              claim verdicts, from{' '}
              <span className="font-medium" data-testid="window-start">{formatDate(data?.window_start ?? null)}</span>{' '}
              to{' '}
              <span className="font-medium" data-testid="window-end">{formatDate(data?.window_end ?? null)}</span>
              . Warned means the athlete saw a warning: a contradicted claim, or an unsupported
              one unless it is training advice. Warnings support later marked as false positives
              are left out, so this counts fewer than Detector health on Claim Verdicts, which reads
              every unsupported or contradicted verdict. A sentence counts as the same
              claim when its words match, so most claims appear once: read this list by date, not
              as a measure of recurrence.
            </p>
          </div>
          <Button onClick={() => refetch()} variant="secondary" disabled={isFetching}>
            {isFetching ? 'Refreshing…' : 'Refresh'}
          </Button>
        </div>

        <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-3">
          <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
            <div className="text-xs text-on-surface-variant">
              Verdicts scanned
            </div>
            <div className="mt-1 text-2xl font-semibold text-on-surface">
              {data?.verdicts_scanned ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
            <div className="text-xs text-on-surface-variant">
              Warned claims
            </div>
            <div className="mt-1 text-2xl font-semibold text-on-surface">
              {data?.flagged_total ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
            <div className="text-xs text-on-surface-variant">
              Distinct claims
            </div>
            <div className="mt-1 text-2xl font-semibold text-on-surface">
              {data?.distinct_claims ?? 0}
            </div>
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
            Failed to load myth-busting summary:{' '}
            {describeAdminError(error)}
          </p>
        </Card>
      ) : data && data.flagged_total === 0 ? (
        <Card className="p-12 text-center">
          <p className="text-on-surface-variant">
            No claims athletes were warned about in the latest scan window.
          </p>
          <p className="mt-1 text-xs text-outline">
            Either agents are well-behaved, or the claim verifier classified
            everything as supported or rhetorical.
          </p>
        </Card>
      ) : data ? (
        <>
          <Card className="overflow-hidden">
            <div className="border-b border-outline-variant px-6 py-3">
              <h3 className="text-sm font-semibold text-on-surface">
                Recent warned claims
              </h3>
              <p className="mt-1 text-xs text-on-surface-variant" data-testid="recent-claims-cap">
                Newest first. Showing {data.recent_claims.length} of {data.distinct_claims} distinct claims.
              </p>
            </div>
            <ul className="divide-y divide-outline-variant">
              {data.recent_claims.map((claim) => (
                <li key={claim.claim_excerpt} className="px-6 py-3">
                  <p className="text-sm text-on-surface">
                    {claim.claim_excerpt}
                  </p>
                  <p className="mt-1 text-xs text-on-surface-variant">
                    {humanizeCategory(claim.status)} · {humanizeCategory(claim.layer_fired)} layer ·{' '}
                    {humanizeCategory(claim.category)} · {plural(claim.occurrences, 'occurrence')} ·{' '}
                    {claim.agent_count === 0 ? 'no agent on record' : plural(claim.agent_count, 'agent')}
                    {' · '}
                    {claim.occurrences > 1 ? `first seen ${formatTimestamp(claim.first_seen_at)} · ` : ''}
                    last seen {formatTimestamp(claim.last_seen_at)}
                  </p>
                </li>
              ))}
            </ul>
          </Card>

          <Card className="overflow-hidden">
            <div className="border-b border-outline-variant px-6 py-3">
              <h3 className="text-sm font-semibold text-on-surface">
                Top offending agents
              </h3>
            </div>
            <ul className="divide-y divide-outline-variant">
              {data.top_agents.map((coach) => (
                <li key={coach.agent_id} className="px-6 py-3">
                  <div className="flex items-start justify-between gap-4">
                    <div className="min-w-0 text-sm text-on-surface">
                      <div className="font-medium">{coach.agent_name ?? 'Agent not found'}</div>
                      <div className="truncate font-mono text-xs text-on-surface-variant">{coach.agent_id}</div>
                    </div>
                    <div className="text-sm font-semibold text-on-surface">
                      {coach.unsupported_total} warned
                    </div>
                  </div>
                  <p className="mt-1 text-xs text-on-surface-variant">
                    Categories: {coach.categories.map(humanizeCategory).join(', ') || '—'}
                  </p>
                </li>
              ))}
            </ul>
          </Card>

          <Card className="overflow-hidden">
            <div className="border-b border-outline-variant px-6 py-3">
              <h3 className="text-sm font-semibold text-on-surface">
                Top categories
              </h3>
            </div>
            <ul className="divide-y divide-outline-variant">
              {data.top_categories.map((cat) => (
                <li
                  key={cat.category}
                  className="flex items-center justify-between gap-4 px-6 py-3"
                >
                  <div className="text-sm font-medium text-on-surface">
                    {humanizeCategory(cat.category)}
                  </div>
                  <div className="text-xs text-on-surface-variant">
                    {cat.flagged_total} warned · {cat.agent_count} agent
                    {cat.agent_count === 1 ? '' : 's'}
                  </div>
                </li>
              ))}
            </ul>
          </Card>
        </>
      ) : null}
    </div>
  );
}
