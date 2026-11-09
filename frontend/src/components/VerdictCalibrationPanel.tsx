// ABOUTME: Verdict calibration panel on the Claim Verdicts screen
// ABOUTME: Pass rate, status mix and daily counts of claim verdicts over the last 30 days
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import {
  adminApi,
  type VerdictCalibrationStats,
  type VerdictStatusBreakdown,
} from '../services/api/admin';
import { useAdminScope } from '../hooks/useAdminScope';
import { Card } from './ui';

/**
 * Verdict pass rate = supported / (supported + unsupported + contradicted).
 * Rhetorical and unverifiable verdicts are excluded because they do not
 * map onto the pass/fail axis — rhetorical claims never reached the
 * evidence layer, and unverifiable claims genuinely lack a ground truth.
 * Returns `null` when the denominator is zero so callers can render a
 * dash instead of `NaN%`.
 */
function verdictPassRate(totals: VerdictStatusBreakdown): number | null {
  const denom = totals.supported + totals.unsupported + totals.contradicted;
  if (denom === 0) return null;
  return totals.supported / denom;
}

function verdictTotalCount(totals: VerdictStatusBreakdown): number {
  return (
    totals.supported +
    totals.unsupported +
    totals.contradicted +
    totals.rhetorical +
    totals.unverifiable
  );
}

/**
 * Verdict calibration panel — surfaces pass rate, status mix, and daily
 * drift for the claim verification pipeline. Sources its data
 * from `GET /admin/evals/verdict-stats` over a 30-day window.
 */
export default function VerdictCalibrationPanel() {
  const { requested } = useAdminScope();
  const { data: stats, isLoading, isError } = useQuery<VerdictCalibrationStats>({
    queryKey: ['admin', 'evals', 'verdict-stats', requested] as const,
    queryFn: () => adminApi.getVerdictCalibrationStats(requested, 30),
  });

  if (isLoading) {
    return (
      <Card className="p-12">
        <div className="flex justify-center">
          <div className="pierre-spinner" />
        </div>
      </Card>
    );
  }

  if (isError || !stats) {
    return (
      <Card className="p-6">
        <h3 className="text-lg font-semibold text-on-surface">
          Verdict calibration
        </h3>
        <p className="mt-2 text-sm text-error">
          Failed to load verdict calibration stats.
        </p>
      </Card>
    );
  }

  const total = verdictTotalCount(stats.totals);
  const passRate = verdictPassRate(stats.totals);
  const passRateLabel = passRate === null ? '—' : `${Math.round(passRate * 100)}%`;
  const maxDailyTotal = stats.daily.reduce(
    (m, d) => Math.max(m, verdictTotalCount(d.counts)),
    0,
  );

  return (
    <div data-testid="verdict-calibration-panel">
    <Card className="p-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="text-lg font-semibold text-on-surface">
            Verdict calibration
          </h3>
          <p className="mt-1 text-sm text-on-surface-variant">
            Claim-verifier verdict mix over the last {stats.window_days} days.
            Pass rate excludes rhetorical and unverifiable claims.
          </p>
        </div>
        <div className="text-right">
          <div className="text-xs text-on-surface-variant">
            Pass rate
          </div>
          <div
            className="text-2xl font-semibold text-on-surface"
            data-testid="calibration-pass-rate"
          >
            {passRateLabel}
          </div>
          <div className="text-xs text-outline">
            {total} verdict{total === 1 ? '' : 's'}
          </div>
        </div>
      </div>

      <div className="mt-4 grid grid-cols-2 gap-2 sm:grid-cols-5">
        <StatusPill label="Supported" count={stats.totals.supported} tone="positive" />
        <StatusPill label="Unsupported" count={stats.totals.unsupported} tone="warning" />
        <StatusPill label="Contradicted" count={stats.totals.contradicted} tone="negative" />
        <StatusPill label="Rhetorical" count={stats.totals.rhetorical} tone="neutral" />
        <StatusPill label="Unverifiable" count={stats.totals.unverifiable} tone="neutral" />
      </div>

      {stats.daily.length === 0 ? (
        <p className="mt-4 text-xs text-outline">
          No verdicts recorded in the selected window.
        </p>
      ) : (
        <div className="mt-5">
          <div className="text-xs text-on-surface-variant">
            Daily drift
          </div>
          <div
            className="mt-2 flex h-24 items-end gap-1"
            data-testid="calibration-daily-drift"
          >
            {stats.daily.map((day) => {
              const dayTotal = verdictTotalCount(day.counts);
              const dayPass = verdictPassRate(day.counts);
              const height = maxDailyTotal === 0 ? 0 : (dayTotal / maxDailyTotal) * 100;
              return (
                <div
                  key={day.date}
                  className="flex-1"
                  title={`${day.date}: ${dayTotal} verdict${
                    dayTotal === 1 ? '' : 's'
                  }, pass rate ${dayPass === null ? '—' : `${Math.round(dayPass * 100)}%`}`}
                >
                  <div
                    className="w-full rounded-t bg-primary"
                    style={{ height: `${Math.max(height, 4)}%` }}
                  />
                </div>
              );
            })}
          </div>
          <div className="mt-1 flex justify-between text-xs text-outline">
            <span>{stats.daily[0]?.date}</span>
            <span>{stats.daily[stats.daily.length - 1]?.date}</span>
          </div>
        </div>
      )}
    </Card>
    </div>
  );
}

interface StatusPillProps {
  label: string;
  count: number;
  tone: 'positive' | 'negative' | 'warning' | 'neutral';
}

function StatusPill({ label, count, tone }: StatusPillProps) {
  const toneClasses: Record<StatusPillProps['tone'], string> = {
    positive: 'text-success border-success',
    negative: 'text-error border-error',
    warning: 'text-on-warning-container border-warning',
    neutral: 'text-on-surface-variant border-outline-variant',
  };
  return (
    <div
      className={`rounded-lg border bg-surface-container-lowest px-3 py-2 ${toneClasses[tone]}`}
    >
      <div className="text-xs">{label}</div>
      <div className="text-lg font-semibold">{count}</div>
    </div>
  );
}
