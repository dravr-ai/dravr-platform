// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Redesigned Overview tab for the Pierre dashboard
// ABOUTME: Features modern stat cards, tier visualization, and admin configuration panel

import { lazy, Suspense } from 'react';
import { useAuth } from '../hooks/useAuth';
import type { DashboardOverview, RateLimitOverview, TierUsage } from '../types/api';
import type { AnalyticsData, TimeSeriesPoint } from '../types/chart';
import { Card, CircularProgress } from './ui';
import { clsx } from 'clsx';

const LazyLineChart = lazy(() => import('react-chartjs-2').then(module => ({ default: module.Line })));

interface OverviewTabProps {
  overview: DashboardOverview | undefined;
  overviewLoading: boolean;
  rateLimits: RateLimitOverview[] | undefined;
  weeklyUsage: AnalyticsData | undefined;
  a2aOverview: { total_clients: number; active_clients: number; requests_today: number; requests_this_month: number } | undefined;
  pendingUsersCount?: number;
  pendingCoachReviews?: number;
  onNavigate?: (tab: string) => void;
}

// Tier colors for visual hierarchy - Dark theme
const tierConfig: Record<string, { color: string; bg: string; border: string; icon: string }> = {
  trial: { color: 'text-on-surface-variant', bg: 'bg-surface-container-low', border: 'ghost-border', icon: 'T' },
  starter: { color: 'text-pierre-activity', bg: 'bg-pierre-activity/10', border: 'border-pierre-activity/30', icon: 'S' },
  professional: { color: 'text-pierre-violet-light', bg: 'bg-pierre-violet/15', border: 'border-pierre-violet/30', icon: 'P' },
  enterprise: { color: 'text-pierre-cyan', bg: 'bg-pierre-cyan/15', border: 'border-pierre-cyan/30', icon: 'E' },
};


export default function OverviewTab({ overview, overviewLoading, rateLimits, weeklyUsage, a2aOverview, pendingUsersCount = 0, pendingCoachReviews = 0, onNavigate }: OverviewTabProps) {
  const { user } = useAuth();

  // Mini chart data
  const miniChartData = {
    labels: weeklyUsage?.time_series?.slice(-7).map((point: TimeSeriesPoint) => {
      const date = new Date(point.date);
      return date.toLocaleDateString('en-US', { weekday: 'short' });
    }) || [],
    datasets: [
      {
        label: 'Requests',
        data: weeklyUsage?.time_series?.slice(-7).map((point: TimeSeriesPoint) => point.request_count) || [],
        borderColor: 'rgb(139, 92, 246)',
        backgroundColor: 'rgba(0, 36, 26, 0.1)',
        tension: 0.4,
        fill: true,
        pointRadius: 3,
      },
    ],
  };

  const miniChartOptions = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: { legend: { display: false } },
    scales: { x: { display: false }, y: { display: false } },
    elements: { point: { hoverRadius: 6 } },
  };

  // Calculate total requests capacity for rate limit visualization
  const totalCapacity = rateLimits?.reduce((sum, item) => sum + (item.limit || 0), 0) || 0;
  const totalUsed = rateLimits?.reduce((sum, item) => sum + item.current_usage, 0) || 0;

  if (overviewLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner"></div>
      </div>
    );
  }

  // Detect when platform has no meaningful data (no connections, no requests)
  const totalConnections = (overview?.total_api_keys || 0) + (a2aOverview?.total_clients || 0);
  const totalRequests = (overview?.total_requests_today || 0) + (overview?.total_requests_this_month || 0) + (a2aOverview?.requests_today || 0) + (a2aOverview?.requests_this_month || 0);
  const isEmptyPlatform = totalConnections === 0 && totalRequests === 0 && pendingUsersCount === 0 && pendingCoachReviews === 0;

  if (isEmptyPlatform) {
    return (
      <div className="space-y-6">
        <Card variant="dark" className="!p-8">
          <div className="text-center py-8">
            <svg className="w-16 h-16 mx-auto mb-6 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M13 10V3L4 14h7v7l9-11h-7z" />
            </svg>
            <h2 className="text-2xl font-bold text-on-surface mb-3">Your platform is ready!</h2>
            <p className="text-on-surface-variant mb-6 max-w-md mx-auto">
              Invite users to get started. Once they connect and start chatting, you will see activity, engagement, and analytics here.
            </p>
            <button
              onClick={() => onNavigate?.('users')}
              className="px-6 py-2.5 rounded-lg bg-pierre-violet/20 text-pierre-violet-light font-medium hover:bg-pierre-violet/30 transition-colors border border-pierre-violet/30"
            >
              Go to Users
            </button>
          </div>
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Hero Stats Row - Dark Theme */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        {/* Total Connections */}
        <div className="stat-card-dark">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-on-surface-variant mb-1">Total Connections</p>
              <p className="text-3xl font-bold bg-gradient-to-r boreal-hero-gradient bg-clip-text text-transparent">
                {(overview?.total_api_keys || 0) + (a2aOverview?.total_clients || 0)}
              </p>
              <p className="text-xs text-outline mt-1">
                {overview?.total_api_keys || 0} Keys + {a2aOverview?.total_clients || 0} Apps
              </p>
            </div>
            <div className="w-12 h-12 rounded-lg bg-gradient-to-br from-pierre-violet/30 to-pierre-cyan/30 flex items-center justify-center">
              <svg className="w-6 h-6 text-pierre-violet-light" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
              </svg>
            </div>
          </div>
        </div>

        {/* Active Connections */}
        <div className="stat-card-dark">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-on-surface-variant mb-1">Active</p>
              <p className="text-3xl font-bold text-pierre-activity">
                {(overview?.active_api_keys || 0) + (a2aOverview?.active_clients || 0)}
              </p>
              <p className="text-xs text-outline mt-1">
                {overview?.active_api_keys || 0} Keys + {a2aOverview?.active_clients || 0} Apps
              </p>
            </div>
            <CircularProgress
              value={(overview?.active_api_keys || 0) + (a2aOverview?.active_clients || 0)}
              max={(overview?.total_api_keys || 1) + (a2aOverview?.total_clients || 0)}
              size="sm"
              variant="activity"
              label="active"
            />
          </div>
        </div>

        {/* Today's Requests */}
        <div className="stat-card-dark">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-on-surface-variant mb-1">Today</p>
              <p className="text-3xl font-bold text-pierre-nutrition">
                {((overview?.total_requests_today || 0) + (a2aOverview?.requests_today || 0)).toLocaleString()}
              </p>
              <p className="text-xs text-outline mt-1">requests</p>
            </div>
            <div className="w-12 h-12 rounded-lg bg-pierre-nutrition/20 flex items-center justify-center">
              <svg className="w-6 h-6 text-pierre-nutrition" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
            </div>
          </div>
        </div>

        {/* Monthly Requests */}
        <div className="stat-card-dark">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-on-surface-variant mb-1">This Month</p>
              <p className="text-3xl font-bold text-pierre-recovery">
                {((overview?.total_requests_this_month || 0) + (a2aOverview?.requests_this_month || 0)).toLocaleString()}
              </p>
              <p className="text-xs text-outline mt-1">requests</p>
            </div>
            <div className="w-12 h-12 rounded-lg bg-pierre-recovery/20 flex items-center justify-center">
              <svg className="w-6 h-6 text-pierre-recovery" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
              </svg>
            </div>
          </div>
        </div>
      </div>

      {/* Two Column Layout */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Weekly Trend */}
        {weeklyUsage?.time_series && weeklyUsage.time_series.length > 0 && (() => {
          const last7Days = weeklyUsage.time_series.slice(-7);
          const totalRequests = last7Days.reduce((sum: number, point: TimeSeriesPoint) => sum + point.request_count, 0);
          // Hide chart entirely when there are no requests — a flat line at 0 is useless
          if (totalRequests === 0) return null;
          const avgPerDay = Math.round(totalRequests / last7Days.length);
          const peakDay = last7Days.reduce((max: TimeSeriesPoint, point: TimeSeriesPoint) =>
            point.request_count > max.request_count ? point : max, last7Days[0]);
          const peakDate = peakDay?.date ? new Date(peakDay.date) : null;
          const peakDayName = peakDate && !isNaN(peakDate.getTime())
            ? peakDate.toLocaleDateString('en-US', { weekday: 'short' })
            : '';

          return (
            <Card variant="dark" className="!p-5">
              <div className="flex justify-between items-center mb-3">
                <div>
                  <h3 className="text-base font-semibold text-on-surface">7-Day Activity</h3>
                  <p className="text-xs text-outline mt-0.5">
                    Avg {avgPerDay.toLocaleString()}/day{peakDayName && ` · Peak ${peakDayName}`}
                  </p>
                </div>
                <span className="px-3 py-1 text-sm font-medium bg-pierre-violet/20 text-pierre-violet-light rounded-full border border-pierre-violet/30">
                  {totalRequests.toLocaleString()} total
                </span>
              </div>
              <div style={{ height: '120px' }}>
                <Suspense fallback={<div className="h-[120px] flex items-center justify-center"><div className="pierre-spinner"></div></div>}>
                  <LazyLineChart data={miniChartData} options={miniChartOptions} />
                </Suspense>
              </div>
            </Card>
          );
        })()}

        {/* Rate Limit Overview */}
        {rateLimits && rateLimits.length > 0 && (
          <Card variant="dark" className="!p-5">
            <div className="flex justify-between items-center mb-4">
              <div>
                <h3 className="text-base font-semibold text-on-surface">Rate Limits</h3>
                <p className="text-xs text-outline mt-0.5">
                  {totalCapacity > 0 ? `${Math.round((totalUsed / totalCapacity) * 100)}% of capacity used` : 'Monitoring usage'}
                </p>
              </div>
              {totalCapacity > 0 && (
                <CircularProgress value={totalUsed} max={totalCapacity} size="md" variant="gradient" />
              )}
            </div>
            <div className="space-y-3 max-h-[200px] overflow-y-auto pr-2 scrollbar-dark">
              {rateLimits.slice(0, 5).map((item: RateLimitOverview) => (
                <div key={item.api_key_id} className="flex items-center gap-3">
                  <div className={clsx(
                    'w-8 h-8 rounded-lg flex items-center justify-center text-xs font-bold',
                    tierConfig[item.tier]?.bg || 'bg-surface-container-low',
                    tierConfig[item.tier]?.color || 'text-on-surface-variant'
                  )}>
                    {tierConfig[item.tier]?.icon || 'T'}
                  </div>
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-medium text-on-surface truncate">{item.api_key_name}</p>
                    <div className="flex items-center gap-2">
                      <div className="flex-1 h-1.5 bg-surface-container-high rounded-full overflow-hidden">
                        <div
                          className={clsx(
                            'h-full rounded-full transition-all duration-300',
                            item.usage_percentage > 90 ? 'bg-pierre-red-500' :
                            item.usage_percentage > 70 ? 'bg-pierre-nutrition' : 'bg-pierre-activity'
                          )}
                          style={{ width: `${Math.min(item.usage_percentage, 100)}%` }}
                        />
                      </div>
                      <span className="text-xs text-outline w-12 text-right">
                        {item.limit ? `${Math.round(item.usage_percentage)}%` : '-'}
                      </span>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </Card>
        )}
      </div>

      {/* Tier Usage Breakdown */}
      {overview?.current_month_usage_by_tier && overview.current_month_usage_by_tier.length > 0 && (
        <Card variant="dark" className="!p-5">
          <h3 className="text-base font-semibold text-on-surface mb-4">Usage by Tier</h3>
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
            {overview.current_month_usage_by_tier.map((tier: TierUsage) => {
              const config = tierConfig[tier.tier] || tierConfig.trial;
              const avgPerKey = tier.key_count > 0 ? Math.round(tier.total_requests / tier.key_count) : 0;
              return (
                <div
                  key={tier.tier}
                  className={clsx(
                    'rounded-lg p-4 border',
                    config.bg,
                    config.border
                  )}
                >
                  <div className="flex items-center gap-2 mb-3">
                    <div className={clsx(
                      'w-8 h-8 rounded-lg flex items-center justify-center text-sm font-bold bg-surface-container-high',
                      config.color
                    )}>
                      {config.icon}
                    </div>
                    <span className={clsx('font-semibold capitalize', config.color)}>{tier.tier}</span>
                  </div>
                  <div className="space-y-1">
                    <div className="flex justify-between text-sm">
                      <span className="text-outline">Keys</span>
                      <span className="font-medium text-on-surface">{tier.key_count}</span>
                    </div>
                    <div className="flex justify-between text-sm">
                      <span className="text-outline">Requests</span>
                      <span className="font-medium text-on-surface">{tier.total_requests.toLocaleString()}</span>
                    </div>
                    <div className="flex justify-between text-sm">
                      <span className="text-outline">Avg/Key</span>
                      <span className="font-medium text-on-surface">{avgPerKey.toLocaleString()}</span>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </Card>
      )}

      {/* Admin Alerts */}
      {user?.is_admin && (
        <div>
          <Card variant="dark" className="!p-4">
            <h3 className="text-sm font-semibold text-on-surface mb-3 flex items-center gap-2">
              <svg className="w-4 h-4 text-pierre-nutrition" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9" />
              </svg>
              Alerts
            </h3>
            <div className="space-y-2">
              {/* Pending Users Alert */}
              {pendingUsersCount > 0 ? (
                <button
                  onClick={() => onNavigate?.('users')}
                  className="w-full flex items-center justify-between p-3 rounded-lg bg-pierre-nutrition/15 border border-pierre-nutrition/30 hover:bg-pierre-nutrition/25 transition-colors"
                >
                  <div className="flex items-center gap-2">
                    <div className="w-2 h-2 rounded-full bg-pierre-nutrition animate-pulse" />
                    <span className="text-sm font-medium text-on-surface">
                      {pendingUsersCount} user{pendingUsersCount !== 1 ? 's' : ''} awaiting approval
                    </span>
                  </div>
                  <svg className="w-4 h-4 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                  </svg>
                </button>
              ) : null}

              {/* Pending Coach Reviews Alert */}
              {pendingCoachReviews > 0 ? (
                <button
                  onClick={() => onNavigate?.('coach-store')}
                  className="w-full flex items-center justify-between p-3 rounded-lg bg-pierre-violet/15 border border-pierre-violet/30 hover:bg-pierre-violet/25 transition-colors"
                >
                  <div className="flex items-center gap-2">
                    <div className="w-2 h-2 rounded-full bg-pierre-violet animate-pulse" />
                    <span className="text-sm font-medium text-on-surface">
                      {pendingCoachReviews} coach{pendingCoachReviews !== 1 ? 'es' : ''} pending review
                    </span>
                  </div>
                  <svg className="w-4 h-4 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                  </svg>
                </button>
              ) : null}

              {/* Rate Limit Warning */}
              {rateLimits?.some((rl: RateLimitOverview) => rl.usage_percentage > 90) ? (
                <button
                  onClick={() => onNavigate?.('analytics')}
                  className="w-full flex items-center justify-between p-3 rounded-lg bg-pierre-red-500/15 border border-pierre-red-500/30 hover:bg-pierre-red-500/25 transition-colors"
                >
                  <div className="flex items-center gap-2">
                    <div className="w-2 h-2 rounded-full bg-pierre-red-500 animate-pulse" />
                    <span className="text-sm font-medium text-on-surface">
                      {rateLimits.filter((rl: RateLimitOverview) => rl.usage_percentage > 90).length} key{rateLimits.filter((rl: RateLimitOverview) => rl.usage_percentage > 90).length !== 1 ? 's' : ''} near limit
                    </span>
                  </div>
                  <svg className="w-4 h-4 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                  </svg>
                </button>
              ) : null}

              {/* All Clear State */}
              {pendingUsersCount === 0 && pendingCoachReviews === 0 && !rateLimits?.some((rl: RateLimitOverview) => rl.usage_percentage > 90) && (
                <div className="flex items-center gap-2 p-3 rounded-lg bg-pierre-activity/15 border border-pierre-activity/30">
                  <svg className="w-4 h-4 text-pierre-activity" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  <span className="text-sm text-on-surface">All systems normal</span>
                </div>
              )}
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}

