// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Overview tab for the admin dashboard with coaching-centric KPI cards
// ABOUTME: Shows active users, conversations, LLM spend, coach adoption, and admin alerts

import { useAuth } from '../hooks/useAuth';
import type { AdminAnalyticsOverview } from '../services/api/admin-analytics';
import { Card } from './ui';

interface OverviewTabProps {
  overview: AdminAnalyticsOverview | undefined;
  overviewLoading: boolean;
  pendingUsersCount?: number;
  pendingCoachReviews?: number;
  onNavigate?: (tab: string) => void;
}

/** Format a number as USD currency string */
function formatUsd(value: number): string {
  if (value >= 1) {
    return `$${value.toFixed(2)}`;
  }
  // Show more precision for sub-dollar amounts
  return `$${value.toFixed(4)}`;
}

export default function OverviewTab({ overview, overviewLoading, pendingUsersCount = 0, pendingCoachReviews = 0, onNavigate }: OverviewTabProps) {
  const { user } = useAuth();

  if (overviewLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner"></div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Hero Stats Row - Coaching-Centric KPIs */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        {/* Active Users (24h) */}
        <div className="stat-card-dark">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-zinc-400 mb-1">Active Users (24h)</p>
              <p className="text-3xl font-bold bg-gradient-to-r from-pierre-violet to-pierre-cyan bg-clip-text text-transparent">
                {overview?.active_users_24h ?? 0}
              </p>
              <p className="text-xs text-zinc-500 mt-1">
                {overview?.active_users_7d ?? 0} active in 7d
              </p>
            </div>
            <div className="w-12 h-12 rounded-lg bg-gradient-to-br from-pierre-violet/30 to-pierre-cyan/30 flex items-center justify-center">
              <svg className="w-6 h-6 text-pierre-violet-light" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z" />
              </svg>
            </div>
          </div>
        </div>

        {/* Conversations Today */}
        <div className="stat-card-dark">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-zinc-400 mb-1">Conversations Today</p>
              <p className="text-3xl font-bold text-pierre-activity">
                {overview?.conversations_today ?? 0}
              </p>
              <p className="text-xs text-zinc-500 mt-1">
                {overview?.messages_today ?? 0} messages
              </p>
            </div>
            <div className="w-12 h-12 rounded-lg bg-pierre-activity/20 flex items-center justify-center">
              <svg className="w-6 h-6 text-pierre-activity" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
              </svg>
            </div>
          </div>
        </div>

        {/* LLM Spend Today */}
        <div className="stat-card-dark">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-zinc-400 mb-1">LLM Spend Today</p>
              <p className="text-3xl font-bold text-pierre-nutrition">
                {formatUsd(overview?.llm_spend_today_usd ?? 0)}
              </p>
              <p className="text-xs text-zinc-500 mt-1">
                {formatUsd(overview?.llm_spend_7d_usd ?? 0)} in 7d
              </p>
            </div>
            <div className="w-12 h-12 rounded-lg bg-pierre-nutrition/20 flex items-center justify-center">
              <svg className="w-6 h-6 text-pierre-nutrition" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V7m0 1v8m0 0v1m0-1c-1.11 0-2.08-.402-2.599-1M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
            </div>
          </div>
        </div>

        {/* Coach Adoption */}
        <div className="stat-card-dark">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-zinc-400 mb-1">Coach Adoption</p>
              <p className="text-3xl font-bold text-pierre-recovery">
                {overview?.coach_adoption_pct ?? 0}%
              </p>
              <p className="text-xs text-zinc-500 mt-1">
                {overview?.connected_providers ?? 0} connected providers
              </p>
            </div>
            <div className="w-12 h-12 rounded-lg bg-pierre-recovery/20 flex items-center justify-center">
              <svg className="w-6 h-6 text-pierre-recovery" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5.121 17.804A13.937 13.937 0 0112 16c2.5 0 4.847.655 6.879 1.804M15 10a3 3 0 11-6 0 3 3 0 016 0zm6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
            </div>
          </div>
        </div>
      </div>

      {/* Admin Alerts */}
      {user?.is_admin && (
        <div>
          <Card variant="dark" className="!p-4">
            <h3 className="text-sm font-semibold text-white mb-3 flex items-center gap-2">
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
                    <span className="text-sm font-medium text-white">
                      {pendingUsersCount} user{pendingUsersCount !== 1 ? 's' : ''} awaiting approval
                    </span>
                  </div>
                  <svg className="w-4 h-4 text-zinc-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
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
                    <span className="text-sm font-medium text-white">
                      {pendingCoachReviews} coach{pendingCoachReviews !== 1 ? 'es' : ''} pending review
                    </span>
                  </div>
                  <svg className="w-4 h-4 text-zinc-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                  </svg>
                </button>
              ) : null}

              {/* All Clear State */}
              {pendingUsersCount === 0 && pendingCoachReviews === 0 && (
                <div className="flex items-center gap-2 p-3 rounded-lg bg-pierre-activity/15 border border-pierre-activity/30">
                  <svg className="w-4 h-4 text-pierre-activity" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  <span className="text-sm text-zinc-300">All systems normal</span>
                </div>
              )}
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}
