// ABOUTME: LLM consumption analytics dashboard panel for admin users
// ABOUTME: Shows token usage, cost, and breakdowns by provider and call type
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import { Line, Doughnut, Bar } from 'react-chartjs-2';
import { usageApi } from '../services/api';
import { useAdminTimeRange, ADMIN_TIME_RANGE_LABELS } from '../hooks/useAdminTimeRange';
import type { AdminTimeRange } from '../hooks/useAdminTimeRange';
import type { LlmConsumptionResponse, ConsumptionBreakdownItem } from '../services/api/usage';
import { QUERY_KEYS } from '../constants/queryKeys';

// Pierre brand colors for charts
const CHART_COLORS = [
  '#00241a', // pierre-violet
  '#0d3b2e', // pierre-cyan
  '#3c6658', // pierre-activity
  '#8f6a2e', // pierre-nutrition
  '#EF4444', // pierre-red
  '#A78BFA', // light violet
  '#FB923C', // orange
  '#5e7a82', // indigo
];

/** Format a token count for display (e.g., 1,234,567 -> "1.23M") */
function formatTokens(count: number): string {
  if (count >= 1_000_000) {
    return `${(count / 1_000_000).toFixed(2)}M`;
  }
  if (count >= 1_000) {
    return `${(count / 1_000).toFixed(1)}K`;
  }
  return count.toLocaleString();
}

/** Format USD cost for display */
function formatCost(usd: number): string {
  if (usd < 0.01) {
    return `$${usd.toFixed(4)}`;
  }
  return `$${usd.toFixed(2)}`;
}

/** Format date for chart labels (e.g. "Feb 17") */
function formatDate(dateStr: string): string {
  const date = new Date(dateStr);
  if (isNaN(date.getTime())) return dateStr;
  return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

/** Group breakdown items by a field and sum their values */
function groupBreakdown(
  items: ConsumptionBreakdownItem[],
  field: 'provider' | 'call_type',
): { labels: string[]; tokens: number[]; calls: number[]; costs: number[] } {
  const grouped = new Map<string, { tokens: number; calls: number; cost: number }>();

  for (const item of items) {
    const key = item[field] || 'unknown';
    const existing = grouped.get(key) ?? { tokens: 0, calls: 0, cost: 0 };
    existing.tokens += item.total_tokens;
    existing.calls += item.calls;
    existing.cost += item.cost_usd;
    grouped.set(key, existing);
  }

  const sorted = [...grouped.entries()].sort((a, b) => b[1].tokens - a[1].tokens);
  return {
    labels: sorted.map(([k]) => k),
    tokens: sorted.map(([, v]) => v.tokens),
    calls: sorted.map(([, v]) => v.calls),
    costs: sorted.map(([, v]) => v.cost),
  };
}

export default function LlmConsumptionPanel() {
  const { timeRange, setTimeRange } = useAdminTimeRange();

  const days = timeRange;

  const { data, isLoading, error } = useQuery<LlmConsumptionResponse>({
    queryKey: [...QUERY_KEYS.usage.llmConsumption(days), 'admin-panel'],
    queryFn: () => usageApi.getAdminLlmConsumption(days),
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
        <div className="text-center py-8 text-pierre-red-400">
          <p className="text-lg mb-2">Failed to load consumption data</p>
          <p className="text-sm text-outline">Check admin permissions and try again.</p>
        </div>
      </div>
    );
  }

  if (!data) {
    return (
      <div className="card-dark">
        <div className="text-center py-8 text-on-surface-variant">
          <p className="text-lg mb-2 text-on-surface">No LLM consumption data</p>
          <p>Start making chat or tool requests to see consumption analytics.</p>
        </div>
      </div>
    );
  }

  const { summary, breakdown, daily_series } = data;

  // Group breakdown by provider and by call_type
  const byProvider = groupBreakdown(breakdown, 'provider');
  const byCallType = groupBreakdown(breakdown, 'call_type');

  // Line chart: daily token consumption time series
  const lineChartData = {
    labels: daily_series.map((p) => formatDate(p.date)),
    datasets: [
      {
        label: 'Tokens',
        data: daily_series.map((p) => p.tokens),
        borderColor: '#00241a',
        backgroundColor: 'rgba(0, 36, 26, 0.1)',
        fill: true,
        tension: 0.4,
        pointRadius: daily_series.length > 30 ? 0 : 3,
        pointHoverRadius: 5,
      },
      {
        label: 'Cost (USD)',
        data: daily_series.map((p) => p.cost_usd),
        borderColor: '#0d3b2e',
        backgroundColor: 'rgba(13, 59, 46, 0.1)',
        fill: false,
        tension: 0.4,
        pointRadius: daily_series.length > 30 ? 0 : 3,
        pointHoverRadius: 5,
        yAxisID: 'y1',
      },
    ],
  };

  const lineChartOptions = {
    responsive: true,
    maintainAspectRatio: false,
    interaction: { mode: 'index' as const, intersect: false },
    plugins: {
      legend: { position: 'bottom' as const, labels: { color: '#a1a1aa' } },
    },
    scales: {
      y: {
        type: 'linear' as const,
        position: 'left' as const,
        title: { display: true, text: 'Tokens', color: '#a1a1aa' },
        ticks: { color: '#a1a1aa' },
        grid: { color: 'rgba(255,255,255,0.1)' },
      },
      y1: {
        type: 'linear' as const,
        position: 'right' as const,
        title: { display: true, text: 'Cost (USD)', color: '#a1a1aa' },
        ticks: { color: '#a1a1aa', callback: (v: string | number) => `$${v}` },
        grid: { drawOnChartArea: false },
      },
      x: {
        ticks: { color: '#a1a1aa', maxRotation: 45 },
        grid: { color: 'rgba(255,255,255,0.1)' },
      },
    },
  };

  // Doughnut chart: breakdown by provider
  const doughnutData = {
    labels: byProvider.labels.map((l) => l.charAt(0).toUpperCase() + l.slice(1)),
    datasets: [
      {
        data: byProvider.tokens,
        backgroundColor: CHART_COLORS.slice(0, byProvider.labels.length),
        borderColor: CHART_COLORS.slice(0, byProvider.labels.length),
        borderWidth: 2,
      },
    ],
  };

  const doughnutOptions = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
      legend: { position: 'bottom' as const, labels: { color: '#a1a1aa' } },
      tooltip: {
        callbacks: {
          label: (ctx: { label: string; raw: unknown }) => {
            const tokens = ctx.raw as number;
            const total = byProvider.tokens.reduce((s, t) => s + t, 0);
            const pct = total > 0 ? ((tokens / total) * 100).toFixed(1) : '0';
            return `${ctx.label}: ${formatTokens(tokens)} tokens (${pct}%)`;
          },
        },
      },
    },
  };

  // Bar chart: breakdown by call_type
  const barChartData = {
    labels: byCallType.labels.map((l) =>
      l.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()),
    ),
    datasets: [
      {
        label: 'Tokens',
        data: byCallType.tokens,
        backgroundColor: 'rgba(0, 36, 26, 0.6)',
        borderColor: '#00241a',
        borderWidth: 1,
      },
      {
        label: 'Calls',
        data: byCallType.calls,
        backgroundColor: 'rgba(13, 59, 46, 0.6)',
        borderColor: '#0d3b2e',
        borderWidth: 1,
      },
    ],
  };

  const barChartOptions = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
      legend: { position: 'bottom' as const, labels: { color: '#a1a1aa' } },
    },
    scales: {
      y: {
        beginAtZero: true,
        ticks: { color: '#a1a1aa' },
        grid: { color: 'rgba(255,255,255,0.1)' },
      },
      x: {
        ticks: { color: '#a1a1aa', maxRotation: 45 },
        grid: { color: 'rgba(255,255,255,0.1)' },
      },
    },
  };

  return (
    <div className="space-y-6">
      {/* Date range selector */}
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold text-on-surface">LLM Consumption Analytics</h2>
        <div className="flex rounded-lg bg-surface-container-high p-1">
          {(Object.entries(ADMIN_TIME_RANGE_LABELS) as [string, string][]).map(([value, label]) => {
            const numValue = Number(value) as AdminTimeRange;
            return (
              <button
                key={value}
                onClick={() => setTimeRange(numValue)}
                className={`px-3 py-1.5 text-sm font-medium rounded-md transition-colors ${
                  timeRange === numValue
                    ? 'bg-pierre-violet text-on-surface shadow-sm'
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
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-pierre-violet">
            {formatTokens(summary.total_tokens)}
          </div>
          <div className="text-sm text-on-surface-variant">Total Tokens</div>
        </div>
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-pierre-cyan">
            {summary.total_calls.toLocaleString()}
          </div>
          <div className="text-sm text-on-surface-variant">Total Calls</div>
        </div>
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-pierre-activity">
            {formatCost(summary.estimated_cost_usd)}
          </div>
          <div className="text-sm text-on-surface-variant">Estimated Cost (USD)</div>
        </div>
      </div>

      {/* Daily consumption time series */}
      {daily_series.length > 0 && (
        <div className="card-dark">
          <h3 className="text-lg font-medium mb-4 text-on-surface">Daily Token Consumption</h3>
          <div style={{ height: '320px' }}>
            <Line data={lineChartData} options={lineChartOptions} />
          </div>
        </div>
      )}

      {/* Breakdown charts: provider + call type */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {byProvider.labels.length > 0 && (
          <div className="card-dark">
            <h3 className="text-lg font-medium mb-4 text-on-surface">By Provider</h3>
            <div style={{ height: '300px' }}>
              <Doughnut data={doughnutData} options={doughnutOptions} />
            </div>
          </div>
        )}

        {byCallType.labels.length > 0 && (
          <div className="card-dark">
            <h3 className="text-lg font-medium mb-4 text-on-surface">By Call Type</h3>
            <div style={{ height: '300px' }}>
              <Bar data={barChartData} options={barChartOptions} />
            </div>
          </div>
        )}
      </div>

      {/* Detailed breakdown table */}
      {breakdown.length > 0 && (
        <div className="card-dark">
          <h3 className="text-lg font-medium mb-4 text-on-surface">Consumption Details</h3>
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-white/10">
              <thead className="bg-surface-container-low">
                <tr>
                  <th className="px-6 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                    Provider
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                    Model
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                    Call Type
                  </th>
                  <th className="px-6 py-3 text-right text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                    Tokens
                  </th>
                  <th className="px-6 py-3 text-right text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                    Calls
                  </th>
                  <th className="px-6 py-3 text-right text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                    Cost
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/10">
                {breakdown.map((item, i) => (
                  <tr key={`${item.provider}-${item.model}-${item.call_type}-${i}`} className="hover:bg-surface-container-low">
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-on-surface">
                      {item.provider}
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-on-surface">
                      {item.model}
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap">
                      <span className={`inline-flex px-2 py-1 text-xs font-medium rounded-full ${
                        item.call_type === 'chat'
                          ? 'bg-pierre-violet/20 text-pierre-violet-light border border-pierre-violet/30'
                          : item.call_type === 'insight'
                          ? 'bg-pierre-cyan/20 text-pierre-cyan-light border border-pierre-cyan/30'
                          : 'bg-surface-container-high text-on-surface border ghost-border'
                      }`}>
                        {item.call_type.replace(/_/g, ' ')}
                      </span>
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-on-surface text-right">
                      {formatTokens(item.total_tokens)}
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-on-surface text-right">
                      {item.calls.toLocaleString()}
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-on-surface text-right">
                      {formatCost(item.cost_usd)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}
