// ABOUTME: Claim-verification admin tab — triage flagged claim verdicts from the bullshit detector
// ABOUTME: Health card, message lookup, status/category/agent/layer/disposition filters, drawer with disposition + knob
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useMemo, useCallback, useEffect } from 'react';
import { useQuery, useQueries, useMutation, useQueryClient } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { adminApi } from '../services/api/admin';
import type { VerdictHealthStats, VerdictKnob } from '../services/api/admin';
import type { ClaimVerdict } from '@pierre/shared-types';
import { CLAIM_VERDICT_LAYERS, VERDICT_DISPOSITIONS } from '@pierre/shared-types';
import { Card, Button, Badge, Select, Input, Section } from './ui';
import VerdictDrawer from './chat/VerdictDrawer';
import VerdictTriagePanel from './VerdictTriagePanel';
import type { DispositionInput } from './VerdictTriagePanel';
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
  { value: 'athlete_data', label: 'Athlete data' },
] as const;

const LAYER_OPTIONS = [
  { value: '', label: 'All layers' },
  ...CLAIM_VERDICT_LAYERS.map((layer) => ({ value: layer, label: humanize(layer) })),
];

const DISPOSITION_OPTIONS = [
  { value: '', label: 'Any disposition' },
  { value: 'undisposed', label: 'Undisposed' },
  ...VERDICT_DISPOSITIONS.map((d) => ({ value: d, label: humanize(d) })),
];

const WINDOW_OPTIONS = [7, 30, 90, 365] as const;

const LIMIT_OPTIONS = [25, 50, 100, 200] as const;

type StatusKey = ClaimVerdict['status'];
type StrengthKey = ClaimVerdict['evidence_strength'];
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

const DISPOSITION_VARIANT: Record<NonNullable<ClaimVerdict['disposition']>, BadgeVariant> = {
  true_catch: 'success',
  false_positive: 'error',
  unsure: 'secondary',
};

function formatTimestamp(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

/** `training_prescription` reads as "Training Prescription" to a human. */
function humanize(value: string): string {
  return value.replace(/_/g, ' ').replace(/\b\w/g, (c: string) => c.toUpperCase());
}

/** `0.75` reads as "75%". */
function formatRate(rate: number): string {
  return `${Math.round(rate * 100)}%`;
}

/** The aggregate health of the detector: how much of what it flags is noise, and where. */
function HealthCard({
  health,
  isLoading,
  windowDays,
  onWindowChange,
}: {
  health: VerdictHealthStats | undefined;
  isLoading: boolean;
  windowDays: number;
  onWindowChange: (days: number) => void;
}) {
  return (
    <Section
      title="Detector health"
      description="Flagged claims over the window and what triage made of them. The rate is false positives over dispositions, so it says nothing until someone has read the flags."
      actions={
        <div className="w-32">
          <Select
            label="Window"
            aria-label="Health window"
            value={String(windowDays)}
            onChange={(e) => onWindowChange(Number(e.target.value))}
            options={WINDOW_OPTIONS.map((d) => ({ value: String(d), label: `${d} days` }))}
          />
        </div>
      }
      data-testid="verdict-health"
    >
      {isLoading || !health ? (
        <div className="flex justify-center p-6">
          <div className="pierre-spinner" />
        </div>
      ) : (
        <>
          <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-6">
            {(
              [
                ['flagged', health.totals.flagged],
                ['disposed', health.totals.disposed],
                ['true catches', health.totals.true_catches],
                ['false positives', health.totals.false_positives],
                ['unsure', health.totals.unsure],
              ] as const
            ).map(([label, count]) => (
              <div
                key={label}
                className="rounded-lg border border-outline-variant bg-surface-container p-3"
              >
                <div className="text-xs text-on-surface-variant">{label}</div>
                <div className="mt-1 text-xl font-semibold text-on-surface">{count}</div>
              </div>
            ))}
            <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
              <div className="text-xs text-on-surface-variant">false-positive rate</div>
              <div
                className="mt-1 text-xl font-semibold text-on-surface"
                data-testid="verdict-health-rate"
              >
                {formatRate(health.false_positive_rate)}
              </div>
            </div>
          </div>

          <div className="mt-6 grid grid-cols-1 gap-6 md:grid-cols-2">
            <div>
              <h3 className="mb-2 text-sm font-semibold text-on-surface">By layer</h3>
              {health.by_layer.length === 0 ? (
                <p className="text-xs text-outline">No flagged claims in the window.</p>
              ) : (
                <table className="min-w-full text-sm" data-testid="verdict-health-by-layer">
                  <thead>
                    <tr className="text-left text-xs text-on-surface-variant">
                      <th className="py-1 pr-3">Layer</th>
                      <th className="py-1 pr-3">Flagged</th>
                      <th className="py-1 pr-3">Disposed</th>
                      <th className="py-1 pr-3">False positives</th>
                      <th className="py-1">Rate</th>
                    </tr>
                  </thead>
                  <tbody>
                    {health.by_layer.map((row) => (
                      <tr key={row.layer} className="border-t border-outline-variant">
                        <td className="py-1 pr-3 text-on-surface">{humanize(row.layer)}</td>
                        <td className="py-1 pr-3 text-on-surface-variant">{row.flagged}</td>
                        <td className="py-1 pr-3 text-on-surface-variant">{row.disposed}</td>
                        <td className="py-1 pr-3 text-on-surface-variant">{row.false_positives}</td>
                        <td className="py-1 text-on-surface">{formatRate(row.rate)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
            <div>
              <h3 className="mb-2 text-sm font-semibold text-on-surface">By reason</h3>
              {health.by_reason.length === 0 ? (
                <p className="text-xs text-outline">No disposition named a reason yet.</p>
              ) : (
                <table className="min-w-full text-sm" data-testid="verdict-health-by-reason">
                  <thead>
                    <tr className="text-left text-xs text-on-surface-variant">
                      <th className="py-1 pr-3">Reason</th>
                      <th className="py-1">Count</th>
                    </tr>
                  </thead>
                  <tbody>
                    {health.by_reason.map((row) => (
                      <tr key={row.reason} className="border-t border-outline-variant">
                        <td className="py-1 pr-3 text-on-surface">{humanize(row.reason)}</td>
                        <td className="py-1 text-on-surface-variant">{row.count}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          </div>
        </>
      )}
    </Section>
  );
}

export default function ClaimVerdictsTab() {
  const { user } = useAuth();
  const tenantId = user?.tenant_id ?? '';
  const queryClient = useQueryClient();

  const [statusFilter, setStatusFilter] = useState<string>('');
  const [categoryFilter, setCategoryFilter] = useState<string>('');
  const [coachFilter, setCoachFilter] = useState<string>('');
  const [layerFilter, setLayerFilter] = useState<string>('');
  const [dispositionFilter, setDispositionFilter] = useState<string>('');
  const [limit, setLimit] = useState<number>(50);
  const [windowDays, setWindowDays] = useState<number>(30);
  const [messageIdInput, setMessageIdInput] = useState<string>('');
  const [lookupMessageId, setLookupMessageId] = useState<string>('');
  /** The rows the drawer shows: the one row clicked, or every row of a looked-up message. */
  const [drawerVerdicts, setDrawerVerdicts] = useState<ClaimVerdict[]>([]);

  const queryKey = useMemo(
    () =>
      [
        'admin',
        'claim-verdicts',
        tenantId,
        statusFilter,
        categoryFilter,
        coachFilter,
        layerFilter,
        dispositionFilter,
        limit,
      ] as const,
    [tenantId, statusFilter, categoryFilter, coachFilter, layerFilter, dispositionFilter, limit],
  );

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey,
    queryFn: () =>
      adminApi.listClaimVerdicts({
        tenant_id: tenantId,
        status: statusFilter || undefined,
        category: categoryFilter || undefined,
        agent_id: coachFilter || undefined,
        layer_fired: layerFilter || undefined,
        disposition: dispositionFilter || undefined,
        limit,
      }),
    enabled: Boolean(tenantId),
  });

  const healthQuery = useQuery({
    queryKey: ['admin', 'claim-verdicts-health', tenantId, windowDays] as const,
    queryFn: () => adminApi.getClaimVerdictHealth(tenantId, windowDays),
    enabled: Boolean(tenantId),
  });

  const lookupQuery = useQuery({
    queryKey: ['admin', 'claim-verdicts-message', tenantId, lookupMessageId] as const,
    queryFn: () => adminApi.listVerdictsForMessage(lookupMessageId, tenantId),
    enabled: Boolean(tenantId) && Boolean(lookupMessageId),
  });

  // The knob is per verdict and lives on the detail read, so the drawer
  // resolves one detail per row it shows. `combine` keeps the map's identity
  // stable while nothing in it changes.
  const knobs = useQueries({
    queries: drawerVerdicts.map((v) => ({
      queryKey: ['admin', 'claim-verdict', tenantId, v.id] as const,
      queryFn: () => adminApi.getClaimVerdict(v.id, tenantId),
      enabled: Boolean(tenantId),
    })),
    combine: (results): Record<string, VerdictKnob | undefined> =>
      Object.fromEntries(drawerVerdicts.map((v, i) => [v.id, results[i]?.data?.knob])),
  });

  const dispositionMutation = useMutation({
    mutationFn: ({ verdict, input }: { verdict: ClaimVerdict; input: DispositionInput }) =>
      adminApi.setClaimVerdictDisposition(verdict.id, {
        tenant_id: tenantId,
        disposition: input.disposition,
        reason: input.reason,
        note: input.note,
      }),
    onSuccess: (detail) => {
      setDrawerVerdicts((rows) => rows.map((r) => (r.id === detail.verdict.id ? detail.verdict : r)));
      void queryClient.invalidateQueries({ queryKey: ['admin', 'claim-verdicts'] });
      void queryClient.invalidateQueries({ queryKey: ['admin', 'claim-verdicts-health'] });
      void queryClient.invalidateQueries({ queryKey: ['admin', 'claim-verdicts-message'] });
      void queryClient.invalidateQueries({
        queryKey: ['admin', 'claim-verdict', tenantId, detail.verdict.id],
      });
    },
  });

  const onSetDisposition = useCallback(
    async (verdict: ClaimVerdict, input: DispositionInput) => {
      await dispositionMutation.mutateAsync({ verdict, input });
    },
    [dispositionMutation],
  );

  const renderTriage = useCallback(
    (verdict: ClaimVerdict) => (
      <VerdictTriagePanel
        verdict={verdict}
        knob={knobs[verdict.id]}
        onSetDisposition={onSetDisposition}
      />
    ),
    [knobs, onSetDisposition],
  );

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

  // A looked-up message opens the drawer on its rows once they land — once
  // per lookup, so closing the drawer does not reopen it — and a message with
  // no verdict says so under the lookup box instead.
  const lookupVerdicts = lookupQuery.data?.verdicts;
  const [openedLookupId, setOpenedLookupId] = useState<string>('');
  useEffect(() => {
    if (!lookupMessageId || !lookupVerdicts || lookupVerdicts.length === 0) return;
    if (openedLookupId === lookupMessageId) return;
    setOpenedLookupId(lookupMessageId);
    setDrawerVerdicts(lookupVerdicts);
  }, [lookupMessageId, lookupVerdicts, openedLookupId]);

  const submitLookup = () => {
    setDrawerVerdicts([]);
    setOpenedLookupId('');
    setLookupMessageId(messageIdInput.trim());
  };

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
      <HealthCard
        health={healthQuery.data}
        isLoading={healthQuery.isLoading}
        windowDays={windowDays}
        onWindowChange={setWindowDays}
      />

      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-on-surface">
              Claim verdicts
            </h2>
            <p className="mt-1 text-sm text-on-surface-variant">
              Every claim an agent makes is evaluated by the claim verifier and
              recorded here. Filter, drill into the source message, record whether
              the flag was right, and course-correct agents that ship unsupported
              or contradicted claims.
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
              <div className="text-xs text-on-surface-variant">
                {key}
              </div>
              <div className="mt-1 text-2xl font-semibold text-on-surface">
                {count}
              </div>
            </div>
          ))}
        </div>
      </Card>

      <Section
        title="Look up a message"
        description="An athlete disputes a reply: paste the message's id to pull every verdict written for it."
      >
        <form
          className="flex flex-col gap-3 sm:flex-row sm:items-end"
          onSubmit={(e) => {
            e.preventDefault();
            submitLookup();
          }}
        >
          <div className="flex-1">
            <Input
              label="Message id"
              type="text"
              value={messageIdInput}
              onChange={(e) => setMessageIdInput(e.target.value)}
              placeholder="paste the disputed message's id"
            />
          </div>
          <Button type="submit" variant="secondary" disabled={!messageIdInput.trim()}>
            Look up
          </Button>
        </form>
        {lookupMessageId && lookupQuery.isError ? (
          <p className="mt-2 text-xs text-error">
            Lookup failed:{' '}
            {lookupQuery.error instanceof Error ? lookupQuery.error.message : String(lookupQuery.error)}
          </p>
        ) : null}
        {lookupMessageId && lookupVerdicts && lookupVerdicts.length === 0 ? (
          <p className="mt-2 text-xs text-on-surface-variant" data-testid="verdict-lookup-empty">
            No verdict was written for message {lookupMessageId}.
          </p>
        ) : null}
      </Section>

      <Card className="p-6">
        <div className="grid grid-cols-1 gap-3 md:grid-cols-3 lg:grid-cols-6">
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
          <Select
            label="Layer"
            value={layerFilter}
            onChange={(e) => setLayerFilter(e.target.value)}
            options={LAYER_OPTIONS}
          />
          <Select
            label="Disposition"
            value={dispositionFilter}
            onChange={(e) => setDispositionFilter(e.target.value)}
            options={DISPOSITION_OPTIONS}
          />
          <Input label="Agent ID" type="text" value={coachFilter} onChange={(e) => setCoachFilter(e.target.value.trim())} placeholder="filter by agent id" />
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
              Verdicts are written when an agent reply passes through the claim
              verifier and produces claims that need evidence.
            </p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-outline-variant">
              <thead className="bg-surface-container">
                <tr>
                  <th className="px-4 py-2 text-left text-xs font-medium text-on-surface-variant">
                    Claim
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-on-surface-variant">
                    Status
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-on-surface-variant">
                    Strength
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-on-surface-variant">
                    Category
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-on-surface-variant">
                    Layer
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-on-surface-variant">
                    Disposition
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-on-surface-variant">
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
                      drawerVerdicts.some((d) => d.id === v.id) && 'bg-primary/10',
                    )}
                    onClick={() => setDrawerVerdicts([v])}
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
                      {humanize(v.category)}
                    </td>
                    <td className="px-4 py-3 text-xs text-on-surface-variant">
                      {v.layer_fired}
                    </td>
                    <td className="px-4 py-3">
                      {v.disposition ? (
                        <Badge variant={DISPOSITION_VARIANT[v.disposition]}>{v.disposition}</Badge>
                      ) : (
                        <span className="text-xs text-outline">—</span>
                      )}
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

      {drawerVerdicts.length > 0 ? (
        <VerdictDrawer
          verdicts={drawerVerdicts}
          onClose={() => setDrawerVerdicts([])}
          renderTriage={renderTriage}
        />
      ) : null}
    </div>
  );
}
