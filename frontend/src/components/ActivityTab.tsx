// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-time activity tab for the admin dashboard showing recent LLM calls and conversations
// ABOUTME: Polls the admin recent-activity endpoint to display live platform usage metrics

import { useQuery } from '@tanstack/react-query';
import { dashboardApi } from '../services/api';
import type { RecentActivityResponse, RecentLlmCall, RecentConversation } from '../services/api/dashboard';
import RealTimeIndicator from './RealTimeIndicator';
import { QUERY_KEYS } from '../constants/queryKeys';

/** Polling interval for real-time activity data (10 seconds) */
const POLLING_INTERVAL_MS = 10_000;

/** Format a duration in ms for display */
function formatDuration(ms: number | null): string {
  if (ms === null) return 'N/A';
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

/** Format USD cost for display */
function formatCost(usd: number): string {
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  return `$${usd.toFixed(2)}`;
}

/** Format tokens with K/M suffixes */
function formatTokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K`;
  return count.toLocaleString();
}

/** Format a relative time (e.g., "2 min ago") */
function formatRelativeTime(isoDate: string): string {
  const now = Date.now();
  const then = new Date(isoDate).getTime();
  const diffSec = Math.floor((now - then) / 1000);

  if (diffSec < 60) return `${diffSec}s ago`;
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
  return new Date(isoDate).toLocaleDateString();
}

export default function ActivityTab() {
  const { data: activity, isLoading } = useQuery<RecentActivityResponse>({
    queryKey: QUERY_KEYS.dashboard.recentActivity(),
    queryFn: () => dashboardApi.getRecentActivity(),
    refetchInterval: POLLING_INTERVAL_MS,
  });

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner w-8 h-8"></div>
      </div>
    );
  }

  const summary = activity?.summary;
  const recentLlm = activity?.recent_llm_calls ?? [];
  const recentConversations = activity?.recent_conversations ?? [];
  const hasData = recentLlm.length > 0 || recentConversations.length > 0;

  if (!hasData && summary?.llm_calls_today === 0) {
    return (
      <div className="space-y-6">
        {/* Summary stats even when empty */}
        <SummaryStats summary={summary} />

        <div className="card-dark">
          <div className="text-center py-12 text-zinc-400">
            <svg className="w-12 h-12 mx-auto mb-4 text-zinc-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M13 10V3L4 14h7v7l9-11h-7z" />
            </svg>
            <p className="text-lg mb-2 text-white">No activity yet</p>
            <p>LLM calls and conversations will appear here in real-time.</p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Summary Stats */}
      <SummaryStats summary={summary} />

      {/* Two-column layout: LLM Calls + Conversations */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Recent LLM Calls */}
        <div className="card-dark">
          <div className="flex justify-between items-center mb-4">
            <h3 className="text-lg font-medium text-white">Recent LLM Calls</h3>
            <RealTimeIndicator className="ml-auto" />
          </div>

          {recentLlm.length === 0 ? (
            <div className="text-center py-6 text-zinc-500">
              <p>No LLM calls recorded yet.</p>
            </div>
          ) : (
            <div className="space-y-2 max-h-96 overflow-y-auto scrollbar-dark">
              {recentLlm.map((call: RecentLlmCall) => (
                <div
                  key={call.id}
                  className="flex items-center justify-between p-3 border border-white/10 rounded-lg hover:bg-white/5 transition-colors"
                >
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-white truncate">
                        {call.provider}/{call.model}
                      </span>
                      <span className="text-xs px-1.5 py-0.5 rounded bg-pierre-violet/20 text-pierre-violet-light">
                        {call.call_type}
                      </span>
                    </div>
                    <div className="flex items-center gap-3 text-xs text-zinc-500 mt-1">
                      <span>{formatTokens(call.total_tokens)} tokens</span>
                      <span>{formatCost(call.cost_usd)}</span>
                      <span>{formatDuration(call.execution_time_ms)}</span>
                    </div>
                  </div>
                  <div className="text-xs text-zinc-500 whitespace-nowrap ml-2">
                    {formatRelativeTime(call.created_at)}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Recent Conversations */}
        <div className="card-dark">
          <div className="flex justify-between items-center mb-4">
            <h3 className="text-lg font-medium text-white">Recent Conversations</h3>
          </div>

          {recentConversations.length === 0 ? (
            <div className="text-center py-6 text-zinc-500">
              <p>No conversations recorded yet.</p>
            </div>
          ) : (
            <div className="space-y-2 max-h-96 overflow-y-auto scrollbar-dark">
              {recentConversations.map((convo: RecentConversation) => (
                <div
                  key={convo.id}
                  className="flex items-center justify-between p-3 border border-white/10 rounded-lg hover:bg-white/5 transition-colors"
                >
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-medium text-white truncate">
                      {convo.title || 'Untitled Conversation'}
                    </p>
                    {convo.user_email && (
                      <p className="text-xs text-zinc-500 mt-0.5 truncate">
                        {convo.user_email}
                      </p>
                    )}
                  </div>
                  <div className="text-xs text-zinc-500 whitespace-nowrap ml-2">
                    {formatRelativeTime(convo.updated_at)}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** Summary stats bar at the top of the activity tab */
function SummaryStats({ summary }: { summary: RecentActivityResponse['summary'] | undefined }) {
  return (
    <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
      <div className="stat-card-dark">
        <div className="text-2xl font-bold text-pierre-cyan">
          {summary?.active_conversations ?? 0}
        </div>
        <div className="text-sm text-zinc-400">Active Conversations (15m)</div>
      </div>
      <div className="stat-card-dark">
        <div className="text-2xl font-bold text-pierre-violet-light">
          {summary?.llm_calls_today ?? 0}
        </div>
        <div className="text-sm text-zinc-400">LLM Calls Today</div>
      </div>
      <div className="stat-card-dark">
        <div className="text-2xl font-bold text-pierre-activity">
          {formatTokens(summary?.total_tokens_today ?? 0)}
        </div>
        <div className="text-sm text-zinc-400">Tokens Today</div>
      </div>
      <div className="stat-card-dark">
        <div className="text-2xl font-bold text-pierre-nutrition">
          {formatCost(summary?.estimated_cost_today ?? 0)}
        </div>
        <div className="text-sm text-zinc-400">Est. Cost Today</div>
      </div>
    </div>
  );
}
