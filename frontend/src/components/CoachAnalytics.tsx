// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach analytics panel for the admin dashboard
// ABOUTME: Shows coach leaderboard, category distribution, and system vs user coach ratio

import { useQuery } from '@tanstack/react-query';
import { adminAnalyticsApi } from '../services/api';
import type { CoachAnalyticsResponse, CoachLeaderboardEntry, CategoryDistribution } from '../services/api/admin-analytics';
import { QUERY_KEYS } from '../constants/queryKeys';
import { clsx } from 'clsx';

// Pierre brand colors for category badges
const CATEGORY_COLORS: Record<string, string> = {
  training: 'bg-pierre-activity/20 text-pierre-activity border-pierre-activity/30',
  nutrition: 'bg-pierre-nutrition/20 text-pierre-nutrition border-pierre-nutrition/30',
  recovery: 'bg-pierre-recovery/20 text-pierre-recovery border-pierre-recovery/30',
  mindset: 'bg-pierre-cyan/20 text-pierre-cyan border-pierre-cyan/30',
  general: 'bg-pierre-violet/20 text-pierre-violet-light border-pierre-violet/30',
};

const DEFAULT_CATEGORY_COLOR = 'bg-white/10 text-zinc-300 border-white/10';

export default function CoachAnalytics() {
  const { data: coaches, isLoading } = useQuery<CoachAnalyticsResponse>({
    queryKey: QUERY_KEYS.adminAnalytics.coaches(),
    queryFn: () => adminAnalyticsApi.getCoaches(),
  });

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner w-8 h-8"></div>
      </div>
    );
  }

  const totalCoaches = (coaches?.system_coach_count ?? 0) + (coaches?.user_coach_count ?? 0);
  const systemPct = totalCoaches > 0
    ? Math.round(((coaches?.system_coach_count ?? 0) / totalCoaches) * 100)
    : 0;

  return (
    <div className="space-y-6">
      <div className="card-admin">
        <h2 className="text-xl font-semibold text-white mb-6">Coach Analytics</h2>

        {/* System vs User Ratio and Category Distribution */}
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 mb-8">
          {/* System vs User Coach Ratio */}
          <div className="bg-white/5 rounded-lg p-5 border border-white/10">
            <h3 className="text-base font-medium text-white mb-4">Coach Distribution</h3>
            <div className="flex items-center gap-4 mb-4">
              <div className="flex-1">
                <div className="flex justify-between text-sm mb-1">
                  <span className="text-pierre-violet-light">System</span>
                  <span className="text-zinc-400">{coaches?.system_coach_count ?? 0}</span>
                </div>
                <div className="h-2 bg-white/10 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-pierre-violet rounded-full transition-all duration-300"
                    style={{ width: `${systemPct}%` }}
                  />
                </div>
              </div>
              <div className="flex-1">
                <div className="flex justify-between text-sm mb-1">
                  <span className="text-pierre-cyan">User-Created</span>
                  <span className="text-zinc-400">{coaches?.user_coach_count ?? 0}</span>
                </div>
                <div className="h-2 bg-white/10 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-pierre-cyan rounded-full transition-all duration-300"
                    style={{ width: `${100 - systemPct}%` }}
                  />
                </div>
              </div>
            </div>
            <p className="text-xs text-zinc-500">{totalCoaches} total coaches</p>
          </div>

          {/* Category Distribution */}
          <div className="bg-white/5 rounded-lg p-5 border border-white/10">
            <h3 className="text-base font-medium text-white mb-4">Categories</h3>
            {coaches?.categories && coaches.categories.length > 0 ? (
              <div className="flex flex-wrap gap-2">
                {coaches.categories.map((cat: CategoryDistribution) => (
                  <span
                    key={cat.category}
                    className={clsx(
                      'inline-flex items-center gap-1.5 px-3 py-1.5 rounded-full text-sm font-medium border',
                      CATEGORY_COLORS[cat.category.toLowerCase()] || DEFAULT_CATEGORY_COLOR
                    )}
                  >
                    {cat.category}
                    <span className="text-xs opacity-70">({cat.count})</span>
                  </span>
                ))}
              </div>
            ) : (
              <p className="text-sm text-zinc-500">No category data available</p>
            )}
          </div>
        </div>

        {/* Coach Leaderboard */}
        <div>
          <h3 className="text-lg font-medium mb-4 text-white">Top Coaches by Usage</h3>
          {coaches?.top_coaches && coaches.top_coaches.length > 0 ? (
            <div className="space-y-3">
              {coaches.top_coaches.map((coach: CoachLeaderboardEntry, index: number) => (
                <div key={coach.coach_id} className="flex justify-between items-center p-3 bg-white/5 rounded-lg hover:bg-white/10 transition-colors border border-white/5">
                  <div className="flex items-center gap-3">
                    <span className={clsx(
                      'w-7 h-7 rounded-full flex items-center justify-center text-xs font-bold',
                      index === 0 ? 'bg-pierre-nutrition/20 text-pierre-nutrition' :
                      index === 1 ? 'bg-zinc-300/20 text-zinc-300' :
                      index === 2 ? 'bg-amber-700/20 text-amber-600' :
                      'bg-white/5 text-zinc-500'
                    )}>
                      {index + 1}
                    </span>
                    <div>
                      <span className="font-medium text-white">{coach.title}</span>
                      <div className="flex items-center gap-2 mt-0.5">
                        <span className={clsx(
                          'text-xs px-1.5 py-0.5 rounded border',
                          CATEGORY_COLORS[coach.category.toLowerCase()] || DEFAULT_CATEGORY_COLOR
                        )}>
                          {coach.category}
                        </span>
                        {coach.is_system && (
                          <span className="text-xs text-zinc-500">System</span>
                        )}
                      </div>
                    </div>
                  </div>
                  <div className="text-right">
                    <div className="font-bold text-pierre-violet-light">{coach.use_count.toLocaleString()}</div>
                    <div className="text-sm text-zinc-500">uses</div>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <div className="text-center py-8 text-zinc-500">
              <p className="text-lg mb-2">No coach usage data yet</p>
              <p>Coach usage will appear here as users interact with coaches</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
