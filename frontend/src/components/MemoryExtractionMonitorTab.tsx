// ABOUTME: Phase B Sprint C6 — admin tab monitoring the memory extraction worker
// ABOUTME: Derives health from user_facts aggregates (total, 24h/7d activity, per-kind breakdown)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  adminApi,
  type ExtractionRunCounts,
  type ExtractionRunStats,
  type MemoryWorkerMetricsResponse,
} from "../services/api/admin";
import { Card, Button } from "./ui";
import { useAdminScope } from "../hooks/useAdminScope";
import AdminScopeBar from "./AdminScopeBar";
import { ADMIN_LOCALE, describeAdminError } from "../utils/adminLocale";

const KIND_LABELS: Record<string, string> = {
  preference: "Preferences",
  physiology: "Physiology",
  injury: "Injuries",
  goal: "Goals",
  schedule: "Schedules",
  equipment: "Equipment",
  other: "Other",
};

function humanizeKind(kind: string): string {
  return KIND_LABELS[kind] ?? kind.charAt(0).toUpperCase() + kind.slice(1);
}

// Largest-remainder (Hamilton) apportionment so percentages always sum to 100.
// Naive Math.round on each row independently can drift to 99% or 101%.
export function distributePercentages(
  counts: number[],
  total: number,
): number[] {
  if (total <= 0 || counts.length === 0) {
    return counts.map(() => 0);
  }
  const exact = counts.map((c) => (c / total) * 100);
  const floors = exact.map((x) => Math.floor(x));
  const remainders = exact.map((x, i) => ({ idx: i, frac: x - floors[i] }));
  let leftover = 100 - floors.reduce((a, b) => a + b, 0);
  remainders.sort((a, b) => b.frac - a.frac);
  const result = [...floors];
  for (let i = 0; i < remainders.length && leftover > 0; i++, leftover--) {
    result[remainders[i].idx] += 1;
  }
  return result;
}

function runIso(ms: number | null): string | null {
  return ms === null ? null : new Date(ms).toISOString();
}

function formatTimestamp(iso: string | null, empty: string): string {
  if (!iso) {
    return empty;
  }
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function relativeFromNow(iso: string | null): string {
  if (!iso) {
    return "—";
  }
  const then = Date.parse(iso);
  if (Number.isNaN(then)) {
    return iso;
  }
  const diffMs = Date.now() - then;
  if (diffMs < 0) {
    return "just now";
  }
  const minutes = Math.round(diffMs / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
}

function totalRuns(counts: ExtractionRunCounts): number {
  return counts.persisted + counts.empty + counts.failed;
}

/**
 * The worker's state, read from its own run ledger. The bands are engineering
 * heuristics: an empty run is healthy (nothing worth keeping was said), and a
 * week with no run at all means no eligible turn happened or the worker is not
 * being reached, which only the operator can tell apart.
 */
export function healthStatus(runs: ExtractionRunStats): {
  label: string;
  variant: "healthy" | "warning" | "idle" | "stalled";
  message: string;
} {
  const day = runs.last_24h;
  const dayTotal = totalRuns(day);
  if (dayTotal > 0 && day.failed === dayTotal) {
    return {
      label: "Failing",
      variant: "stalled",
      message: `All ${dayTotal} runs in the last 24 hours failed. Check the worker's errors and the LLM provider.`,
    };
  }
  if (day.failed > 0) {
    return {
      label: "Degraded",
      variant: "warning",
      message: `${day.failed} of ${dayTotal} runs in the last 24 hours failed. A failed run is retried by the resume sweep.`,
    };
  }
  if (runs.backlog > 0) {
    return {
      label: "Backlog",
      variant: "warning",
      message: `${runs.backlog} extractions are still owed past a run's lease, waiting for the resume sweep.`,
    };
  }
  if (dayTotal > 0) {
    return {
      label: "Healthy",
      variant: "healthy",
      message:
        "Runs in the last 24 hours finished. A run that found nothing to keep counts as finished.",
    };
  }
  if (totalRuns(runs.last_7d) > 0) {
    return {
      label: "Quiet",
      variant: "idle",
      message:
        "No run in the last 24 hours, some in the past week. Expected when nobody is chatting.",
    };
  }
  return {
    label: "Idle",
    variant: "idle",
    message:
      "No run in the last 7 days: no eligible turn happened, or the worker is not being reached.",
  };
}

// A token tint tracks the theme on its own, so these no longer need a dark:
// variant to restate the same intent in a second palette.
const STATUS_CLASS: Record<ReturnType<typeof healthStatus>["variant"], string> =
  {
    healthy: "bg-success/15 text-on-success-container",
    warning: "bg-warning/15 text-on-warning-container",
    idle: "bg-surface-container-high text-on-surface-variant",
    stalled: "bg-error/15 text-error",
  };

export default function MemoryExtractionMonitorTab() {
  const { requested } = useAdminScope();

  const { data, isLoading, isError, error, refetch, isFetching } =
    useQuery<MemoryWorkerMetricsResponse>({
      queryKey: ["admin", "memory-worker-metrics", requested] as const,
      queryFn: () => adminApi.getMemoryWorkerMetrics(requested),
      refetchInterval: 60_000,
    });

  const metrics = data?.metrics;
  const runs = data?.runs;

  const kindRows = useMemo(() => {
    if (!metrics) {
      return [] as { kind: string; count: number; pct: number }[];
    }
    const sorted = Object.entries(metrics.facts_by_kind)
      .map(([kind, count]) => ({ kind, count }))
      .sort((a, b) => b.count - a.count);
    const pcts = distributePercentages(
      sorted.map((r) => r.count),
      metrics.total_facts,
    );
    return sorted.map((row, i) => ({ ...row, pct: pcts[i] }));
  }, [metrics]);

  return (
    <div className="space-y-4">
      <AdminScopeBar scope={data?.scope} />
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-on-surface">
              Memory extraction worker
            </h2>
            <p className="mt-1 text-sm text-on-surface-variant">
              Every post-turn extraction records how it ended, so the state
              below is measured from the worker&apos;s runs, including those
              that found nothing to keep. The{" "}
              <code className="rounded bg-surface-container px-1 py-0.5 text-xs">
                user_facts
              </code>{" "}
              figures describe the store, which athlete approvals and agent
              tools also write to, so they say nothing about the worker.
            </p>
          </div>
          <Button
            onClick={() => refetch()}
            variant="secondary"
            disabled={isFetching}
          >
            {isFetching ? "Refreshing…" : "Refresh"}
          </Button>
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
            Failed to load memory worker metrics: {describeAdminError(error)}
          </p>
        </Card>
      ) : metrics && runs ? (
        <>
          <Card className="p-6">
            {(() => {
              const status = healthStatus(runs);
              return (
                <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                  <div className="flex items-center gap-3">
                    <span
                      className={`rounded-full px-3 py-1 text-sm font-semibold ${STATUS_CLASS[status.variant]}`}
                    >
                      {status.label}
                    </span>
                    <p className="text-sm text-on-surface-variant">
                      {status.message}
                    </p>
                  </div>
                  <div className="text-sm text-on-surface-variant">
                    Last run{" "}
                    <span className="font-medium text-on-surface">
                      {relativeFromNow(runIso(runs.last_run_at_ms))}
                    </span>
                    <span className="ml-2 text-xs text-outline">
                      (
                      {formatTimestamp(
                        runIso(runs.last_run_at_ms),
                        "No run recorded",
                      )}
                      )
                    </span>
                  </div>
                </div>
              );
            })()}
          </Card>

          <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
            {[
              ["Runs, last 24 hours", totalRuns(runs.last_24h)],
              ["Failed, last 24 hours", runs.last_24h.failed],
              ["Facts written, last 24 hours", runs.last_24h.facts_persisted],
              ["Owed past the lease", runs.backlog],
            ].map(([label, value]) => (
              <Card key={label} className="p-4">
                <div className="text-xs text-on-surface-variant">{label}</div>
                <div className="mt-1 text-3xl font-semibold text-on-surface">
                  {value.toLocaleString(ADMIN_LOCALE)}
                </div>
              </Card>
            ))}
          </div>

          <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
            <Card className="p-4">
              <div className="text-xs text-on-surface-variant">Total facts</div>
              <div className="mt-1 text-3xl font-semibold text-on-surface">
                {metrics.total_facts.toLocaleString()}
              </div>
            </Card>
            <Card className="p-4">
              <div className="text-xs text-on-surface-variant">
                Fact rows touched, 24 hours
              </div>
              <div className="mt-1 text-3xl font-semibold text-on-surface">
                {metrics.facts_last_24h.toLocaleString()}
              </div>
            </Card>
            <Card className="p-4">
              <div className="text-xs text-on-surface-variant">
                Fact rows touched, 7 days
              </div>
              <div className="mt-1 text-3xl font-semibold text-on-surface">
                {metrics.facts_last_7d.toLocaleString()}
              </div>
            </Card>
            <Card className="p-4">
              <div className="text-xs text-on-surface-variant">
                Distinct users
              </div>
              <div className="mt-1 text-3xl font-semibold text-on-surface">
                {metrics.distinct_users.toLocaleString()}
              </div>
            </Card>
          </div>

          <Card className="overflow-hidden">
            <div className="border-b border-outline-variant px-6 py-3">
              <h3 className="text-sm font-semibold text-on-surface">
                Breakdown by fact kind
              </h3>
              <p className="text-xs text-on-surface-variant">
                Extraction distribution across semantic categories — heavy skew
                toward{" "}
                <code className="rounded bg-surface-container px-1">other</code>{" "}
                usually means the extractor prompt needs tuning.
              </p>
            </div>
            {kindRows.length === 0 ? (
              <div className="p-6 text-sm text-on-surface-variant">
                No facts stored yet.
              </div>
            ) : (
              <ul className="divide-y divide-outline-variant">
                {kindRows.map((row) => {
                  const pct = row.pct;
                  return (
                    <li
                      key={row.kind}
                      className="flex items-center gap-4 px-6 py-3"
                    >
                      <div className="w-32 text-sm font-medium text-on-surface">
                        {humanizeKind(row.kind)}
                      </div>
                      <div className="flex-1">
                        <div className="h-2 overflow-hidden rounded-full bg-surface-container">
                          <div
                            className="h-full bg-primary"
                            style={{ width: `${pct}%` }}
                          />
                        </div>
                      </div>
                      <div className="w-24 text-right text-sm text-on-surface-variant">
                        {row.count.toLocaleString()}{" "}
                        <span className="text-xs text-outline">({pct}%)</span>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </Card>
        </>
      ) : null}
    </div>
  );
}
