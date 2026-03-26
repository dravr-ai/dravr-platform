// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Conversation analytics panel for the admin dashboard
// ABOUTME: Shows daily conversation/message charts, peak hours, and avg messages per conversation

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { adminAnalyticsApi } from '../services/api';
import type { ConversationAnalyticsResponse, ConversationDayPoint } from '../services/api/admin-analytics';
import {
  Chart as ChartJS,
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  Title,
  Tooltip,
  Legend,
  Filler,
} from 'chart.js';
import { Line } from 'react-chartjs-2';
import { QUERY_KEYS } from '../constants/queryKeys';

ChartJS.register(
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  Title,
  Tooltip,
  Legend,
  Filler
);

// Pierre Design System colors for charts
const PIERRE_COLORS = {
  violet: '#8B5CF6',
  cyan: '#22D3EE',
  activity: '#4ADE80',
};

/** Format a peak hour number (0-23) into a human-readable time label */
function formatPeakHour(hour: number): string {
  const period = hour >= 12 ? 'PM' : 'AM';
  const displayHour = hour % 12 || 12;
  return `${displayHour}:00 ${period}`;
}

export default function UsageAnalytics() {
  const [timeRange, setTimeRange] = useState<number>(7);

  const { data: analytics, isLoading } = useQuery<ConversationAnalyticsResponse>({
    queryKey: QUERY_KEYS.adminAnalytics.conversations(timeRange),
    queryFn: () => adminAnalyticsApi.getConversations(timeRange),
  });

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

  // Prepare dual-axis chart data: conversations + messages
  const chartData = {
    labels: analytics?.daily?.map((point: ConversationDayPoint) => formatDateLabel(point.date)) || [],
    datasets: [
      {
        label: 'Conversations',
        data: analytics?.daily?.map((point: ConversationDayPoint) => point.conversations) || [],
        borderColor: PIERRE_COLORS.violet,
        backgroundColor: `${PIERRE_COLORS.violet}1A`,
        tension: 0.4,
        fill: true,
        yAxisID: 'y',
      },
      {
        label: 'Messages',
        data: analytics?.daily?.map((point: ConversationDayPoint) => point.messages) || [],
        borderColor: PIERRE_COLORS.cyan,
        backgroundColor: `${PIERRE_COLORS.cyan}1A`,
        tension: 0.4,
        fill: true,
        yAxisID: 'y1',
      },
    ],
  };

  const chartOptions = {
    responsive: true,
    interaction: {
      mode: 'index' as const,
      intersect: false,
    },
    plugins: {
      legend: {
        position: 'top' as const,
        labels: { color: '#a1a1aa' },
      },
    },
    scales: {
      x: {
        ticks: { color: '#71717a' },
        grid: { color: 'rgba(255, 255, 255, 0.05)' },
      },
      y: {
        type: 'linear' as const,
        display: true,
        position: 'left' as const,
        beginAtZero: true,
        ticks: { color: '#71717a' },
        grid: { color: 'rgba(255, 255, 255, 0.05)' },
        title: {
          display: true,
          text: 'Conversations',
          color: '#71717a',
        },
      },
      y1: {
        type: 'linear' as const,
        display: true,
        position: 'right' as const,
        beginAtZero: true,
        ticks: { color: '#71717a' },
        grid: { drawOnChartArea: false },
        title: {
          display: true,
          text: 'Messages',
          color: '#71717a',
        },
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
          <h2 className="text-xl font-semibold text-white">Conversation Analytics</h2>
          <select
            value={timeRange}
            onChange={(e) => setTimeRange(Number(e.target.value))}
            className="select-dark w-auto"
          >
            <option value={7}>Last 7 days</option>
            <option value={30}>Last 30 days</option>
            <option value={90}>Last 90 days</option>
          </select>
        </div>

        {/* Summary Stats */}
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-8">
          <div className="stat-card-dark">
            <div className="text-2xl font-bold text-pierre-violet-light">
              {analytics?.total_conversations ?? 0}
            </div>
            <div className="text-sm text-zinc-400">Total Conversations</div>
          </div>
          <div className="stat-card-dark">
            <div className="text-2xl font-bold text-pierre-cyan">
              {analytics?.avg_messages_per_conversation?.toFixed(1) ?? '0.0'}
            </div>
            <div className="text-sm text-zinc-400">Avg Messages / Conversation</div>
          </div>
          <div className="stat-card-dark">
            <div className="text-2xl font-bold text-pierre-activity">
              {analytics?.peak_hour !== undefined ? formatPeakHour(analytics.peak_hour) : 'N/A'}
            </div>
            <div className="text-sm text-zinc-400">Peak Hour</div>
          </div>
        </div>

        {/* Daily Conversations + Messages Chart */}
        <div className="mb-8">
          <h3 className="text-lg font-medium mb-4 text-white">Conversations &amp; Messages Over Time</h3>
          <div className="bg-white/5 rounded-lg p-4 border border-white/10">
            {analytics?.daily && analytics.daily.length > 0 ? (
              <Line data={chartData} options={chartOptions} />
            ) : (
              <div className="bg-white/5 rounded-lg p-8 text-center text-zinc-500">
                No conversation data available yet
                <br />
                <small>Start coaching conversations to see analytics here</small>
              </div>
            )}
          </div>
        </div>

        {(!analytics?.daily?.length) && (
          <div className="text-center py-8 text-zinc-500">
            <p className="text-lg mb-2">No conversation data yet</p>
            <p>Users need to start coaching conversations to see analytics</p>
          </div>
        )}
      </div>
    </div>
  );
}
