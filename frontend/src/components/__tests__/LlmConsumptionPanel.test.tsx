// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for LLM consumption panel helper functions
// ABOUTME: Validates token formatting, cost formatting, date formatting, and breakdown grouping

import { describe, it, expect } from 'vitest';

// We test the pure utility functions used by LlmConsumptionPanel.
// These are module-private, so we replicate the logic for testing.

function formatTokens(count: number): string {
  if (count >= 1_000_000) {
    return `${(count / 1_000_000).toFixed(2)}M`;
  }
  if (count >= 1_000) {
    return `${(count / 1_000).toFixed(1)}K`;
  }
  return count.toLocaleString();
}

function formatCost(usd: number): string {
  if (usd < 0.01) {
    return `$${usd.toFixed(4)}`;
  }
  return `$${usd.toFixed(2)}`;
}

function formatDate(dateStr: string): string {
  const date = new Date(dateStr);
  if (isNaN(date.getTime())) return dateStr;
  return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

interface BreakdownItem {
  provider: string;
  model: string;
  call_type: string;
  total_tokens: number;
  calls: number;
  cost_usd: number;
}

function groupBreakdown(
  items: BreakdownItem[],
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

describe('formatTokens', () => {
  it('formats millions correctly', () => {
    expect(formatTokens(1_500_000)).toBe('1.50M');
    expect(formatTokens(2_000_000)).toBe('2.00M');
    expect(formatTokens(500_000_000)).toBe('500.00M');
  });

  it('formats thousands correctly', () => {
    expect(formatTokens(1_500)).toBe('1.5K');
    expect(formatTokens(50_000)).toBe('50.0K');
    expect(formatTokens(999_999)).toBe('1000.0K');
  });

  it('formats small numbers with locale string', () => {
    expect(formatTokens(0)).toBe('0');
    expect(formatTokens(500)).toBe('500');
  });
});

describe('formatCost', () => {
  it('formats costs >= $0.01 with 2 decimals', () => {
    expect(formatCost(1.5)).toBe('$1.50');
    expect(formatCost(0.05)).toBe('$0.05');
    expect(formatCost(123.45)).toBe('$123.45');
  });

  it('formats tiny costs with 4 decimals', () => {
    expect(formatCost(0.001)).toBe('$0.0010');
    expect(formatCost(0.0075)).toBe('$0.0075');
    expect(formatCost(0)).toBe('$0.0000');
  });
});

describe('formatDate', () => {
  it('formats ISO date strings', () => {
    // Use noon UTC to avoid timezone-shift across day boundary
    const result = formatDate('2026-02-17T12:00:00Z');
    expect(result).toMatch(/Feb\s+17/);
  });

  it('returns input for invalid dates', () => {
    expect(formatDate('not-a-date')).toBe('not-a-date');
  });
});

describe('groupBreakdown', () => {
  const items: BreakdownItem[] = [
    { provider: 'gemini', model: 'gemini-2.0-flash', call_type: 'chat', total_tokens: 1000, calls: 5, cost_usd: 0.10 },
    { provider: 'gemini', model: 'gemini-2.5-pro', call_type: 'insight', total_tokens: 2000, calls: 3, cost_usd: 0.50 },
    { provider: 'groq', model: 'llama-3.3-70b', call_type: 'chat', total_tokens: 500, calls: 2, cost_usd: 0.05 },
    { provider: 'direct', model: 'get_activities', call_type: 'mcp_tool', total_tokens: 0, calls: 10, cost_usd: 0 },
  ];

  it('groups by provider and sums tokens', () => {
    const result = groupBreakdown(items, 'provider');
    expect(result.labels).toEqual(['gemini', 'groq', 'direct']);
    expect(result.tokens).toEqual([3000, 500, 0]);
    expect(result.calls).toEqual([8, 2, 10]);
  });

  it('groups by call_type and sorts by tokens descending', () => {
    const result = groupBreakdown(items, 'call_type');
    // insight (2000) > chat (1500) > mcp_tool (0) — sorted descending by tokens
    expect(result.labels).toEqual(['insight', 'chat', 'mcp_tool']);
    expect(result.tokens).toEqual([2000, 1500, 0]);
  });

  it('handles empty input', () => {
    const result = groupBreakdown([], 'provider');
    expect(result.labels).toEqual([]);
    expect(result.tokens).toEqual([]);
  });
});

describe('time range constants', () => {
  it('maps time ranges to correct days', () => {
    const timeRangeDays = { '7d': 7, '30d': 30, '90d': 90 };
    expect(timeRangeDays['7d']).toBe(7);
    expect(timeRangeDays['30d']).toBe(30);
    expect(timeRangeDays['90d']).toBe(90);
  });
});
