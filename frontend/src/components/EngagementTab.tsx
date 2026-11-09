// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Engagement tab for the admin dashboard showing agent leaderboard and user engagement
// ABOUTME: Every figure follows the admin scope; agents ranked by uses, users by last activity; the range moves only Requests

import { useQuery } from "@tanstack/react-query";
import { adminApi } from "../services/api";
import { useAdminScope } from "../hooks/useAdminScope";
import {
  useAdminTimeRange,
  ADMIN_TIME_RANGE_LABELS,
} from "../hooks/useAdminTimeRange";
import type { AdminTimeRange } from "../hooks/useAdminTimeRange";
import AdminScopeBar from "./AdminScopeBar";
import { usageAnalyticsQuery } from "./analyticsQueries";
import { Card } from "./ui";
import { ADMIN_LOCALE } from "../utils/adminLocale";

interface EngagementTabProps {
  onNavigate?: (tab: string) => void;
}

export default function EngagementTab({ onNavigate }: EngagementTabProps) {
  const { timeRange, setTimeRange } = useAdminTimeRange();
  const { requested } = useAdminScope();

  // Fetch usage analytics for the time series data
  const { data: analytics, isLoading: analyticsLoading } = useQuery(
    usageAnalyticsQuery(timeRange, requested),
  );

  // Every other figure, read in the same scope as the series
  const { data: engagement, isLoading: engagementLoading } = useQuery({
    queryKey: ["admin", "engagement", requested] as const,
    queryFn: () => adminApi.getEngagement(requested),
  });

  const isLoading = analyticsLoading || engagementLoading;

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner w-8 h-8"></div>
      </div>
    );
  }

  // Cumulative tiers, computed by the server: DAU ⊆ WAU ⊆ MAU.
  const tiers = engagement?.users ?? {
    total: 0,
    daily: 0,
    weekly: 0,
    monthly: 0,
    dormant: 0,
  };
  const dailyActive = tiers.daily;
  const weeklyActive = tiers.weekly;
  const monthlyActive = tiers.monthly;
  const dormant = tiers.dormant;

  // The server ranks the leaderboard by uses, a tie by title, and keeps the top 20.
  const sortedCoaches = engagement?.leaderboard ?? [];
  const totalCoaches = engagement?.total_agents ?? 0;
  const publishedCount = engagement?.published_agents ?? 0;

  // Check if there is any data at all
  const hasEngagementData = tiers.total > 0 || totalCoaches > 0;

  if (!hasEngagementData) {
    return (
      <div className="space-y-6">
        <TimeRangeSelector timeRange={timeRange} onChange={setTimeRange} />
        <Card variant="dark">
          <div className="text-center py-12 text-on-surface-variant">
            <svg
              className="w-12 h-12 mx-auto mb-4 text-on-surface-variant"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.5}
                d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z"
              />
            </svg>
            <p className="text-lg mb-2 text-on-surface">
              No agent activity yet
            </p>
            <p className="mb-6">
              Create system agents to get started. Engagement data will appear
              as users interact with agents.
            </p>
            <button
              onClick={() => onNavigate?.("agents")}
              className="px-6 py-2.5 rounded-lg bg-primary/20 text-primary font-medium hover:bg-primary/30 transition-colors border border-primary/30"
            >
              Go to Agents
            </button>
          </div>
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Time Range Selector */}
      <TimeRangeSelector timeRange={timeRange} onChange={setTimeRange} />

      {/* Every figure on this page reads through the one scope rule. */}
      <div className="space-y-1">
        <p className="text-xs text-on-surface-variant">
          This scope applies to every figure. The time range moves only the
          Requests figure; everything else reads as of now.
        </p>
        <AdminScopeBar scope={engagement?.scope ?? analytics?.scope} />
      </div>

      {/* User Engagement Tiers */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
        <div className="stat-card-dark">
          <div className="font-mono text-2xl font-medium text-on-surface">
            {dailyActive}
          </div>
          <div className="text-sm text-on-surface-variant">
            Active in the last 24h
          </div>
          <div className="text-xs text-on-surface-variant">
            Users by last activity
          </div>
        </div>
        <div className="stat-card-dark">
          <div className="font-mono text-2xl font-medium text-on-surface">
            {weeklyActive}
          </div>
          <div className="text-sm text-on-surface-variant">
            Active in the last 7d
          </div>
          <div className="text-xs text-on-surface-variant">
            Users by last activity
          </div>
        </div>
        <div className="stat-card-dark">
          <div className="font-mono text-2xl font-medium text-on-surface">
            {monthlyActive}
          </div>
          <div className="text-sm text-on-surface-variant">
            Active in the last 30d
          </div>
          <div className="text-xs text-on-surface-variant">
            Users by last activity
          </div>
        </div>
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-outline">{dormant}</div>
          <div className="text-sm text-on-surface-variant">Dormant</div>
          <div className="text-xs text-on-surface-variant">
            Not active in 30d
          </div>
        </div>
      </div>

      {/* Two-column layout: Agent Leaderboard + Platform Stats */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Agent Leaderboard — 2/3 width */}
        <div className="lg:col-span-2 card-dark">
          <h2 className="text-lg font-medium text-on-surface">
            Agent Leaderboard
          </h2>
          <p className="mt-1 mb-4 text-xs text-on-surface-variant">
            The scope above, all time. A use is a pick or a conversation started
            with the agent, so a messaging session that rolls over counts again.
          </p>
          {sortedCoaches.length === 0 ? (
            <div className="text-center py-6 text-outline">
              <p>No agents created yet.</p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="min-w-full divide-y divide-outline-variant">
                <thead className="bg-surface-container-low">
                  <tr>
                    <th className="px-4 py-3 text-left text-xs font-medium text-on-surface-variant">
                      #
                    </th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-on-surface-variant">
                      Agent
                    </th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-on-surface-variant">
                      Category
                    </th>
                    <th className="px-4 py-3 text-right text-xs font-medium text-on-surface-variant">
                      Uses
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-outline-variant">
                  {sortedCoaches.map((coach, index) => (
                    <tr
                      key={coach.id}
                      className="hover:bg-surface-container-low"
                    >
                      <td className="px-4 py-3 whitespace-nowrap text-sm text-outline">
                        {index + 1}
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap">
                        <p className="text-sm font-medium text-on-surface">
                          {coach.title}
                        </p>
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap">
                        <span className="text-xs px-2 py-1 rounded bg-primary/20 text-primary">
                          {coach.category || "general"}
                        </span>
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap text-sm text-on-surface text-right">
                        {coach.use_count.toLocaleString(ADMIN_LOCALE)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>

        {/* Platform Stats — 1/3 width */}
        <div className="space-y-4">
          <Card variant="dark">
            <h2 className="text-lg font-medium mb-4 text-on-surface">
              Platform Stats
            </h2>
            <div className="space-y-4">
              <StatRow label="Total Users" value={tiers.total} />
              <StatRow label="Total Agents" value={totalCoaches} />
              <StatRow label="Published Agents" value={publishedCount} />
              <StatRow
                label={`Requests (${ADMIN_TIME_RANGE_LABELS[timeRange]})`}
                value={
                  analytics?.time_series?.reduce(
                    (sum, p) => sum + p.request_count,
                    0,
                  ) ?? 0
                }
              />
            </div>
          </Card>
        </div>
      </div>
    </div>
  );
}

/** Time range selector component */
function TimeRangeSelector({
  timeRange,
  onChange,
}: {
  timeRange: AdminTimeRange;
  onChange: (v: AdminTimeRange) => void;
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-sm text-on-surface-variant">Requests over:</span>
      <div className="flex gap-1">
        {(Object.entries(ADMIN_TIME_RANGE_LABELS) as [string, string][]).map(
          ([value, label]) => {
            const numValue = Number(value) as AdminTimeRange;
            return (
              <button
                key={value}
                onClick={() => onChange(numValue)}
                className={`px-3 py-1.5 text-sm rounded-lg transition-colors ${
                  timeRange === numValue
                    ? "bg-primary/20 text-primary border border-primary/30"
                    : "text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface border border-transparent"
                }`}
              >
                {label}
              </button>
            );
          },
        )}
      </div>
    </div>
  );
}

/** Simple stat row for the platform stats card */
function StatRow({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-on-surface-variant">{label}</span>
      <span className="text-sm font-medium text-on-surface">
        {value.toLocaleString(ADMIN_LOCALE)}
      </span>
    </div>
  );
}
