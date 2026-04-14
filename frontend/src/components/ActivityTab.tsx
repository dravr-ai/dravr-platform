// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-time activity tab for the admin dashboard showing LLM calls and conversations
// ABOUTME: Unified searchable table with filters, expandable rows, and live polling

import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { dashboardApi } from '../services/api';
import type { RecentActivityResponse, RecentLlmCall, RecentConversation } from '../services/api/dashboard';
import RealTimeIndicator from './RealTimeIndicator';
import { QUERY_KEYS } from '../constants/queryKeys';
import { Input } from './ui';

/** Polling interval for real-time activity data (10 seconds) */
const POLLING_INTERVAL_MS = 10_000;

type ActivityType = 'all' | 'llm' | 'conversation';

/** Unified activity entry for the combined table */
interface ActivityEntry {
  id: string;
  type: 'llm' | 'conversation';
  title: string;
  subtitle: string;
  category: string;
  detail: string;
  timestamp: string;
  rawData: RecentLlmCall | RecentConversation;
}

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

/** Convert API data into unified activity entries */
function buildEntries(
  llmCalls: RecentLlmCall[],
  conversations: RecentConversation[],
): ActivityEntry[] {
  const llmEntries: ActivityEntry[] = llmCalls.map((call) => ({
    id: `llm-${call.id}`,
    type: 'llm' as const,
    title: `${call.provider}/${call.model}`,
    subtitle: `${formatTokens(call.total_tokens)} tokens  ${formatCost(call.cost_usd)}  ${formatDuration(call.execution_time_ms)}`,
    category: call.call_type,
    detail: `Provider: ${call.provider} | Model: ${call.model} | Tokens: ${call.total_tokens.toLocaleString()} | Cost: ${formatCost(call.cost_usd)} | Duration: ${formatDuration(call.execution_time_ms)}`,
    timestamp: call.created_at,
    rawData: call,
  }));

  const convoEntries: ActivityEntry[] = conversations.map((convo) => ({
    id: `convo-${convo.id}`,
    type: 'conversation' as const,
    title: convo.title || 'Untitled Conversation',
    subtitle: convo.user_email || '',
    category: 'conversation',
    detail: `User: ${convo.user_email || 'Unknown'} | Title: ${convo.title || 'Untitled'}`,
    timestamp: convo.updated_at,
    rawData: convo,
  }));

  return [...llmEntries, ...convoEntries].sort(
    (a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime(),
  );
}

export default function ActivityTab() {
  const [searchQuery, setSearchQuery] = useState('');
  const [typeFilter, setTypeFilter] = useState<ActivityType>('all');
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const { data: activity, isLoading } = useQuery<RecentActivityResponse>({
    queryKey: QUERY_KEYS.dashboard.recentActivity(),
    queryFn: () => dashboardApi.getRecentActivity(),
    refetchInterval: POLLING_INTERVAL_MS,
  });

  const summary = activity?.summary;
  const recentLlm = useMemo(() => activity?.recent_llm_calls ?? [], [activity?.recent_llm_calls]);
  const recentConversations = useMemo(() => activity?.recent_conversations ?? [], [activity?.recent_conversations]);

  // Build unified entries from both data sources
  const allEntries = useMemo(
    () => buildEntries(recentLlm, recentConversations),
    [recentLlm, recentConversations],
  );

  // Apply filters
  const filteredEntries = useMemo(() => {
    let entries = allEntries;

    // Type filter
    if (typeFilter !== 'all') {
      entries = entries.filter((e) => e.type === typeFilter);
    }

    // Search filter
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      entries = entries.filter(
        (e) =>
          e.title.toLowerCase().includes(q) ||
          e.subtitle.toLowerCase().includes(q) ||
          e.category.toLowerCase().includes(q),
      );
    }

    return entries;
  }, [allEntries, typeFilter, searchQuery]);

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner w-8 h-8"></div>
      </div>
    );
  }

  const hasData = recentLlm.length > 0 || recentConversations.length > 0;

  return (
    <div className="space-y-6">
      {/* Summary Stats */}
      <SummaryStats summary={summary} />

      {/* Filters Bar */}
      <div className="flex items-center gap-4 flex-wrap">
        {/* Type filter chips */}
        <div className="flex items-center gap-2">
          {(['all', 'llm', 'conversation'] as const).map((type) => {
            const labels: Record<ActivityType, string> = {
              all: `All (${allEntries.length})`,
              llm: `LLM Calls (${allEntries.filter((e) => e.type === 'llm').length})`,
              conversation: `Conversations (${allEntries.filter((e) => e.type === 'conversation').length})`,
            };
            return (
              <button
                key={type}
                onClick={() => setTypeFilter(type)}
                className={`px-3 py-1.5 rounded-full text-sm font-medium transition-colors ${
                  typeFilter === type
                    ? 'bg-pierre-violet text-on-surface'
                    : 'bg-surface-container-high text-on-surface hover:bg-surface-container-highest'
                }`}
              >
                {labels[type]}
              </button>
            );
          })}
        </div>

        {/* Search */}
        <div className="flex-1 max-w-sm ml-auto">
          <Input
            type="search"
            placeholder="Search by provider, user, model..."
            aria-label="Search activity"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
        </div>

        <RealTimeIndicator />
      </div>

      {/* Activity Table */}
      {!hasData && summary?.llm_calls_today === 0 ? (
        <div className="card-dark">
          <div className="text-center py-12 text-on-surface-variant">
            <svg className="w-12 h-12 mx-auto mb-4 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M13 10V3L4 14h7v7l9-11h-7z" />
            </svg>
            <p className="text-lg mb-2 text-on-surface">No activity yet</p>
            <p>LLM calls and conversations will appear here in real-time.</p>
          </div>
        </div>
      ) : (
        <div className="card-dark">
          {/* Table header */}
          <div className="grid grid-cols-12 gap-3 px-4 py-3 border-b ghost-border text-xs font-medium text-on-surface-variant uppercase tracking-wider">
            <div className="col-span-1">Type</div>
            <div className="col-span-4">Details</div>
            <div className="col-span-2">Category</div>
            <div className="col-span-3">Metrics</div>
            <div className="col-span-2 text-right">Time</div>
          </div>

          {/* Rows */}
          <div className="max-h-[600px] overflow-y-auto scrollbar-dark">
            {filteredEntries.length === 0 ? (
              <div className="text-center py-8 text-outline">
                {searchQuery ? 'No results match your search.' : 'No activity in this category.'}
              </div>
            ) : (
              filteredEntries.map((entry) => (
                <div key={entry.id}>
                  <button
                    onClick={() => setExpandedId(expandedId === entry.id ? null : entry.id)}
                    className="w-full grid grid-cols-12 gap-3 px-4 py-3 border-b ghost-border hover:bg-surface-container-low transition-colors text-left items-center"
                  >
                    {/* Type icon */}
                    <div className="col-span-1">
                      {entry.type === 'llm' ? (
                        <span className="inline-flex items-center justify-center w-8 h-8 rounded-lg bg-pierre-violet/20">
                          <svg className="w-4 h-4 text-pierre-violet-light" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
                          </svg>
                        </span>
                      ) : (
                        <span className="inline-flex items-center justify-center w-8 h-8 rounded-lg bg-pierre-cyan/20">
                          <svg className="w-4 h-4 text-pierre-cyan" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
                          </svg>
                        </span>
                      )}
                    </div>

                    {/* Title + subtitle */}
                    <div className="col-span-4 min-w-0">
                      <p className="text-sm font-medium text-on-surface truncate">{entry.title}</p>
                      <p className="text-xs text-outline truncate mt-0.5">{entry.subtitle}</p>
                    </div>

                    {/* Category badge */}
                    <div className="col-span-2">
                      <span className={`text-xs px-2 py-1 rounded-full ${
                        entry.type === 'llm'
                          ? 'bg-pierre-violet/20 text-pierre-violet-light'
                          : 'bg-pierre-cyan/20 text-pierre-cyan'
                      }`}>
                        {entry.category}
                      </span>
                    </div>

                    {/* Metrics */}
                    <div className="col-span-3 text-xs text-on-surface-variant">
                      {entry.type === 'llm' ? (
                        <div className="flex items-center gap-3">
                          <span>{formatTokens((entry.rawData as RecentLlmCall).total_tokens)}</span>
                          <span>{formatCost((entry.rawData as RecentLlmCall).cost_usd)}</span>
                          <span>{formatDuration((entry.rawData as RecentLlmCall).execution_time_ms)}</span>
                        </div>
                      ) : (
                        <span className="text-outline">{(entry.rawData as RecentConversation).user_email}</span>
                      )}
                    </div>

                    {/* Timestamp */}
                    <div className="col-span-2 text-right">
                      <span className="text-xs text-outline">{formatRelativeTime(entry.timestamp)}</span>
                      <svg
                        className={`w-4 h-4 inline-block ml-2 text-on-surface-variant transition-transform ${
                          expandedId === entry.id ? 'rotate-180' : ''
                        }`}
                        fill="none" stroke="currentColor" viewBox="0 0 24 24"
                      >
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                      </svg>
                    </div>
                  </button>

                  {/* Expanded detail row */}
                  {expandedId === entry.id && (
                    <div className="px-4 py-3 bg-surface-container-low border-b ghost-border">
                      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
                        {entry.type === 'llm' ? (
                          <>
                            <DetailItem label="Provider" value={(entry.rawData as RecentLlmCall).provider} />
                            <DetailItem label="Model" value={(entry.rawData as RecentLlmCall).model} />
                            <DetailItem label="Tokens" value={(entry.rawData as RecentLlmCall).total_tokens.toLocaleString()} />
                            <DetailItem label="Cost" value={formatCost((entry.rawData as RecentLlmCall).cost_usd)} />
                            <DetailItem label="Duration" value={formatDuration((entry.rawData as RecentLlmCall).execution_time_ms)} />
                            <DetailItem label="Type" value={(entry.rawData as RecentLlmCall).call_type} />
                            <DetailItem label="Time" value={new Date(entry.timestamp).toLocaleString()} />
                          </>
                        ) : (
                          <>
                            <DetailItem label="Title" value={(entry.rawData as RecentConversation).title || 'Untitled'} />
                            <DetailItem label="User" value={(entry.rawData as RecentConversation).user_email || 'Unknown'} />
                            <DetailItem label="Last Updated" value={new Date(entry.timestamp).toLocaleString()} />
                          </>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              ))
            )}
          </div>

          {/* Footer with count */}
          <div className="px-4 py-2 border-t ghost-border text-xs text-outline">
            Showing {filteredEntries.length} of {allEntries.length} entries
          </div>
        </div>
      )}
    </div>
  );
}

/** Detail item in expanded row */
function DetailItem({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span className="text-outline">{label}</span>
      <p className="text-on-surface mt-0.5 font-mono text-xs">{value}</p>
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
        <div className="text-sm text-on-surface-variant">Active Conversations (15m)</div>
      </div>
      <div className="stat-card-dark">
        <div className="text-2xl font-bold text-pierre-violet-light">
          {summary?.llm_calls_today ?? 0}
        </div>
        <div className="text-sm text-on-surface-variant">LLM Calls Today</div>
      </div>
      <div className="stat-card-dark">
        <div className="text-2xl font-bold text-pierre-activity">
          {formatTokens(summary?.total_tokens_today ?? 0)}
        </div>
        <div className="text-sm text-on-surface-variant">Tokens Today</div>
      </div>
      <div className="stat-card-dark">
        <div className="text-2xl font-bold text-pierre-nutrition">
          {formatCost(summary?.estimated_cost_today ?? 0)}
        </div>
        <div className="text-sm text-on-surface-variant">Est. Cost Today</div>
      </div>
    </div>
  );
}
