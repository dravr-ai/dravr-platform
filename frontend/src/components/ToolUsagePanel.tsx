// ABOUTME: Tool-usage analytics dashboard panel for admin users
// ABOUTME: Shows which MCP/coach tools run, how often, across how many turns, and avg latency
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import { usageApi } from '../services/api';
import { useAdminTimeRange, ADMIN_TIME_RANGE_LABELS } from '../hooks/useAdminTimeRange';
import type { AdminTimeRange } from '../hooks/useAdminTimeRange';
import type { ToolUsageResponse } from '../services/api/usage';

/** Format an average latency (ms) for display, or a dash when unknown. */
function formatLatency(ms: number | null): string {
  if (ms === null) return '—';
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${ms}ms`;
}

export default function ToolUsagePanel() {
  const { timeRange, setTimeRange } = useAdminTimeRange();
  const days = timeRange;

  const { data, isLoading, error } = useQuery<ToolUsageResponse>({
    queryKey: ['admin', 'tool-usage', days],
    queryFn: () => usageApi.getAdminToolUsage(days),
    // Degrade gracefully — never propagate to the error boundary (which would
    // blank the whole Analytics tab if this endpoint is unavailable).
    throwOnError: false,
    retry: false,
  });

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner w-8 h-8"></div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="card-dark">
        <div className="text-center py-8 text-error">
          <p className="text-lg mb-2">Failed to load tool-usage data</p>
          <p className="text-sm text-outline">Check admin permissions and try again.</p>
        </div>
      </div>
    );
  }

  // Defensive: an unexpected response shape (e.g. SPA HTML fallback when the
  // endpoint is unavailable) must never throw and blank the Analytics tab.
  if (!data || !Array.isArray(data.breakdown) || data.breakdown.length === 0) {
    return (
      <div className="card-dark">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-semibold text-on-surface">Tool Usage</h3>
        </div>
        <div className="text-center py-8 text-on-surface-variant">
          <p className="text-lg mb-2 text-on-surface">No tool usage yet</p>
          <p>Once the agent runs tools in chat, they show up here.</p>
        </div>
      </div>
    );
  }

  const breakdown = data.breakdown;
  const summary = data.summary ?? {
    total_invocations: 0,
    unique_tools: 0,
    turns_with_tools: 0,
  };

  return (
    <div className="card-dark">
      <div className="flex flex-wrap items-center justify-between gap-3 mb-4">
        <h3 className="text-lg font-semibold text-on-surface">Tool Usage</h3>
        <div className="flex gap-1 bg-surface-container rounded-lg p-1">
          {(Object.entries(ADMIN_TIME_RANGE_LABELS) as [string, string][]).map(([value, label]) => {
            const numValue = Number(value) as AdminTimeRange;
            return (
              <button
                key={value}
                onClick={() => setTimeRange(numValue)}
                className={`px-3 py-1.5 text-sm font-medium rounded-md transition-colors ${
                  timeRange === numValue
                    ? 'bg-primary text-on-primary'
                    : 'text-on-surface-variant hover:text-on-surface'
                }`}
              >
                {label}
              </button>
            );
          })}
        </div>
      </div>

      {/* Summary cards */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 mb-6">
        <div className="bg-surface-container rounded-lg p-4">
          <p className="text-sm text-on-surface-variant">Total invocations</p>
          <p className="text-2xl font-semibold text-on-surface">
            {summary.total_invocations.toLocaleString()}
          </p>
        </div>
        <div className="bg-surface-container rounded-lg p-4">
          <p className="text-sm text-on-surface-variant">Unique tools</p>
          <p className="text-2xl font-semibold text-on-surface">{summary.unique_tools}</p>
        </div>
        <div className="bg-surface-container rounded-lg p-4">
          <p className="text-sm text-on-surface-variant">Turns with tools</p>
          <p className="text-2xl font-semibold text-on-surface">
            {summary.turns_with_tools.toLocaleString()}
          </p>
        </div>
      </div>

      {/* Per-tool breakdown */}
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-on-surface-variant border-b border-outline-variant">
              <th className="py-2 pr-4 font-medium">Tool</th>
              <th className="py-2 px-4 font-medium text-right">Invocations</th>
              <th className="py-2 px-4 font-medium text-right">Turns</th>
              <th className="py-2 pl-4 font-medium text-right">Avg latency</th>
            </tr>
          </thead>
          <tbody>
            {breakdown.map((item) => (
              <tr
                key={item.tool_name}
                className="border-b border-outline-variant/50 hover:bg-surface-container-low"
              >
                <td className="py-2 pr-4 font-mono text-on-surface">{item.tool_name}</td>
                <td className="py-2 px-4 text-right text-on-surface">
                  {item.invocation_count.toLocaleString()}
                </td>
                <td className="py-2 px-4 text-right text-on-surface-variant">
                  {item.turn_count.toLocaleString()}
                </td>
                <td className="py-2 pl-4 text-right text-on-surface-variant">
                  {formatLatency(item.avg_latency_ms)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
