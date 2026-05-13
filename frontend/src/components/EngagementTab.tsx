// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Engagement tab for the admin dashboard showing coach leaderboard and user engagement
// ABOUTME: Displays coach usage rankings, user activity tiers, provider stats, and group metrics

import { useQuery } from '@tanstack/react-query';
import { adminApi, dashboardApi } from '../services/api';
import { useAdminTimeRange, ADMIN_TIME_RANGE_LABELS } from '../hooks/useAdminTimeRange';
import type { AdminTimeRange } from '../hooks/useAdminTimeRange';
import { QUERY_KEYS } from '../constants/queryKeys';
import type { AnalyticsData } from '../types/chart';
import { Card } from './ui';

interface EngagementTabProps {
  onNavigate?: (tab: string) => void;
}

export default function EngagementTab({ onNavigate }: EngagementTabProps) {
  const { timeRange, setTimeRange } = useAdminTimeRange();

  // Fetch usage analytics for the time series data
  const { data: analytics, isLoading: analyticsLoading } = useQuery<AnalyticsData>({
    queryKey: QUERY_KEYS.dashboard.usageAnalytics(timeRange),
    queryFn: () => dashboardApi.getUsageAnalytics(timeRange),
  });

  // Fetch system coaches for the leaderboard
  const { data: coachesData, isLoading: coachesLoading } = useQuery({
    queryKey: QUERY_KEYS.adminCoaches.system(),
    queryFn: () => adminApi.getSystemCoaches(),
  });

  // Fetch store stats for engagement overview
  const { data: storeStats } = useQuery({
    queryKey: QUERY_KEYS.adminStore.stats(),
    queryFn: () => adminApi.getStoreStats(),
  });

  // Fetch all users to compute engagement tiers
  const { data: allUsers, isLoading: usersLoading } = useQuery({
    queryKey: [...QUERY_KEYS.adminUsers.all, 'engagement-all'],
    queryFn: () => adminApi.getAllUsers(),
  });

  const isLoading = analyticsLoading || coachesLoading || usersLoading;

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner w-8 h-8"></div>
      </div>
    );
  }

  // Compute engagement tiers from user data
  const users = allUsers || [];
  const now = Date.now();
  const dayMs = 86_400_000;

  // Cumulative buckets: DAU ⊆ WAU ⊆ MAU. Mutually exclusive bins
  // (the prior implementation) produced impossible values like
  // DAU=2 > WAU=1 > MAU=0 because each user could only land in one
  // bucket — readers expect industry-standard cumulative counters.
  let dailyActive = 0;
  let weeklyActive = 0;
  let monthlyActive = 0;
  let dormant = 0;

  for (const user of users) {
    const lastActive = user.last_active ? new Date(user.last_active).getTime() : 0;
    const daysSinceActive = (now - lastActive) / dayMs;

    if (daysSinceActive <= 1) dailyActive++;
    if (daysSinceActive <= 7) weeklyActive++;
    if (daysSinceActive <= 30) monthlyActive++;
    if (daysSinceActive > 30) dormant++;
  }

  // Build coach leaderboard sorted by token_count (proxy for usage)
  const coaches = coachesData?.coaches || [];
  const sortedCoaches = [...coaches]
    .sort((a, b) => (b.token_count ?? 0) - (a.token_count ?? 0))
    .slice(0, 20);

  const totalCoaches = coaches.length;
  const publishedCount = storeStats?.published_count ?? 0;

  // Check if there is any data at all
  const hasEngagementData = users.length > 0 || coaches.length > 0;

  if (!hasEngagementData) {
    return (
      <div className="space-y-6">
        <TimeRangeSelector timeRange={timeRange} onChange={setTimeRange} />
        <Card variant="dark">
          <div className="text-center py-12 text-on-surface-variant">
            <svg className="w-12 h-12 mx-auto mb-4 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z" />
            </svg>
            <p className="text-lg mb-2 text-on-surface">No coach activity yet</p>
            <p className="mb-6">Create system coaches to get started. Engagement data will appear as users interact with coaches.</p>
            <button
              onClick={() => onNavigate?.('coaches')}
              className="px-6 py-2.5 rounded-lg bg-pierre-violet/20 text-pierre-violet-light font-medium hover:bg-pierre-violet/30 transition-colors border border-pierre-violet/30"
            >
              Go to Coaches
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

      {/* User Engagement Tiers */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-pierre-activity">{dailyActive}</div>
          <div className="text-sm text-on-surface-variant">Daily Active</div>
          <div className="text-[10px] text-on-surface-variant">Last 24h</div>
        </div>
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-pierre-cyan">{weeklyActive}</div>
          <div className="text-sm text-on-surface-variant">Weekly Active</div>
          <div className="text-[10px] text-on-surface-variant">Last 7d</div>
        </div>
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-pierre-violet-light">{monthlyActive}</div>
          <div className="text-sm text-on-surface-variant">Monthly Active</div>
          <div className="text-[10px] text-on-surface-variant">Last 30d</div>
        </div>
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-outline">{dormant}</div>
          <div className="text-sm text-on-surface-variant">Dormant</div>
        </div>
      </div>

      {/* Two-column layout: Coach Leaderboard + Platform Stats */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Coach Leaderboard — 2/3 width */}
        <div className="lg:col-span-2 card-dark">
          <h3 className="text-lg font-medium mb-4 text-on-surface">Coach Leaderboard</h3>
          {sortedCoaches.length === 0 ? (
            <div className="text-center py-6 text-outline">
              <p>No coaches created yet.</p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="min-w-full divide-y divide-white/10">
                <thead className="bg-surface-container-low">
                  <tr>
                    <th className="px-4 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">#</th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">Coach</th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">Category</th>
                    <th className="px-4 py-3 text-right text-xs font-medium text-on-surface-variant uppercase tracking-wider" title="Coach system-prompt size (chars), proxy for setup cost — not real per-coach consumption">Prompt Size</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-white/10">
                  {sortedCoaches.map((coach, index) => (
                    <tr key={coach.id} className="hover:bg-surface-container-low">
                      <td className="px-4 py-3 whitespace-nowrap text-sm text-outline">
                        {index + 1}
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap">
                        <p className="text-sm font-medium text-on-surface">{coach.title}</p>
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap">
                        <span className="text-xs px-2 py-1 rounded bg-pierre-violet/20 text-pierre-violet-light">
                          {coach.category || 'general'}
                        </span>
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap text-sm text-on-surface text-right">
                        {(coach.token_count ?? 0).toLocaleString()}
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
            <h3 className="text-lg font-medium mb-4 text-on-surface">Platform Stats</h3>
            <div className="space-y-4">
              <StatRow label="Total Users" value={users.length} />
              <StatRow label="Total Coaches" value={totalCoaches} />
              <StatRow label="Published Coaches" value={publishedCount} />
              <StatRow
                label={`Requests (${ADMIN_TIME_RANGE_LABELS[timeRange]})`}
                value={analytics?.time_series?.reduce((sum, p) => sum + p.request_count, 0) ?? 0}
              />
            </div>
          </Card>
        </div>
      </div>
    </div>
  );
}

/** Time range selector component */
function TimeRangeSelector({ timeRange, onChange }: { timeRange: AdminTimeRange; onChange: (v: AdminTimeRange) => void }) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-sm text-on-surface-variant">Time Range:</span>
      <div className="flex gap-1">
        {(Object.entries(ADMIN_TIME_RANGE_LABELS) as [string, string][]).map(([value, label]) => {
          const numValue = Number(value) as AdminTimeRange;
          return (
            <button
              key={value}
              onClick={() => onChange(numValue)}
              className={`px-3 py-1.5 text-sm rounded-lg transition-colors ${
                timeRange === numValue
                  ? 'bg-pierre-violet/20 text-pierre-violet-light border border-pierre-violet/30'
                  : 'text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface border border-transparent'
              }`}
            >
              {label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** Simple stat row for the platform stats card */
function StatRow({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-on-surface-variant">{label}</span>
      <span className="text-sm font-medium text-on-surface">{value.toLocaleString()}</span>
    </div>
  );
}
