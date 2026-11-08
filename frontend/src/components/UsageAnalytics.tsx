// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Usage analytics dashboard showing request volume and latency
// ABOUTME: Reads shared admin time range from localStorage for consistent cross-tab state

import { useQuery } from '@tanstack/react-query';
import { usageAnalyticsQuery } from './analyticsQueries';
import { useAdminScope } from '../hooks/useAdminScope';
import { useAdminTimeRange, ADMIN_TIME_RANGE_LABELS } from '../hooks/useAdminTimeRange';
import type { AdminTimeRange } from '../hooks/useAdminTimeRange';
import type { ChartData, ChartOptions, TimeSeriesPoint } from '../types/chart';
import '../utils/registerChartJs';
import { tint, useChartTheme } from '../hooks/useChartTheme';
import { Line } from 'react-chartjs-2';

export default function UsageAnalytics() {
  const { timeRange, setTimeRange } = useAdminTimeRange();
  const { requested } = useAdminScope();

  const { data: analytics, isLoading, isError } = useQuery(usageAnalyticsQuery(timeRange, requested));

  const chart = useChartTheme();

  // Helper to safely parse dates from various formats
  const formatDateLabel = (dateString: string): string => {
    if (!dateString) return 'N/A';

    // Try parsing as ISO date first
    let date = new Date(dateString);

    // If invalid, try adding time component for date-only strings (YYYY-MM-DD)
    if (isNaN(date.getTime()) && /^\d{4}-\d{2}-\d{2}$/.test(dateString)) {
      date = new Date(`${dateString}T00:00:00`);
    }

    // If still invalid, return the original string or a fallback
    if (isNaN(date.getTime())) {
      return dateString.length > 10 ? dateString.substring(0, 10) : dateString;
    }

    return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  };

  // Prepare chart data
  const timeSeriesData: ChartData = {
    labels: analytics?.time_series?.map((point: TimeSeriesPoint) => formatDateLabel(point.date)) || [],
    datasets: [
      {
        label: 'API Requests',
        data: analytics?.time_series?.map((point: TimeSeriesPoint) => point.request_count) || [],
        borderColor: chart.series[0],
        backgroundColor: tint(chart.series[0], 0.1),
        tension: 0.4,
        fill: true,
      },
    ],
  };

  const chartOptions: ChartOptions = {
    responsive: true,
    plugins: {
      legend: {
        position: 'top',
        labels: { color: chart.text },
      },
    },
    scales: {
      x: {
        ticks: { color: chart.text },
        grid: { color: chart.grid },
      },
      y: {
        beginAtZero: true,
        ticks: { color: chart.text },
        grid: { color: chart.grid },
      },
    },
  };

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner w-8 h-8"></div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="card-admin">
        <div className="flex justify-between items-center mb-6">
          <h2 className="text-xl font-semibold text-on-surface">Usage Analytics</h2>
          <div className="flex rounded-lg bg-surface-container-high p-1">
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

        {isError && !analytics ? (
          <p role="alert" className="py-8 text-center text-sm text-error">
            Usage analytics could not be loaded. These numbers will return once the server answers.
          </p>
        ) : (
        <>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-8">
          <div className="stat-card-dark">
            <div className="font-mono text-2xl font-medium text-on-surface">
              {analytics?.time_series?.reduce((sum: number, point: TimeSeriesPoint) => sum + point.request_count, 0) || 0}
            </div>
            <div className="text-sm text-on-surface-variant">Total Requests</div>
          </div>
          <div className="stat-card-dark">
            <div className="font-mono text-2xl font-medium text-on-surface">
              {analytics?.average_response_time?.toFixed(0) || 0}ms
            </div>
            <div className="text-sm text-on-surface-variant">Avg Response Time</div>
          </div>
        </div>

        {/* Time Series Chart */}
        <div className="mb-8">
          <h3 className="text-lg font-medium mb-4 text-on-surface">Request Volume Over Time</h3>
          <div className="bg-surface-container-low rounded-lg p-4 border ghost-border">
            {analytics?.time_series && analytics.time_series.length > 0 ? (
              <Line data={timeSeriesData} options={chartOptions} />
            ) : (
              <div className="bg-surface-container-low rounded-lg p-8 text-center text-outline">
                No time series data available yet
                <br />
                <small>Make some API calls to see request patterns</small>
              </div>
            )}
          </div>
        </div>

        {!analytics?.time_series?.length && (
          <div className="text-center py-12 text-on-surface-variant">
            <svg className="w-12 h-12 mx-auto mb-4 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M7 12l3-3 3 3 4-4M8 21l4-4 4 4M3 4h18M4 4h16v12a1 1 0 01-1 1H5a1 1 0 01-1-1V4z" />
            </svg>
            <p className="text-lg mb-2 text-on-surface">No conversations yet</p>
            <p>Users will see analytics here once they start chatting.</p>
          </div>
        )}
        </>
        )}
      </div>

    </div>
  );
}