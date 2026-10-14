// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The admin Analytics tab: usage, LLM consumption and tool usage, revealed together once their data settles
// ABOUTME: Shown one by one, each panel landing above the next would push it down — a layout shift of ~1.0

import { useState } from 'react';
import { useQueries } from '@tanstack/react-query';
import { useAdminTimeRange } from '../hooks/useAdminTimeRange';
import { usageAnalyticsQuery, llmConsumptionQuery, toolUsageQuery } from './analyticsQueries';
import UsageAnalytics from './UsageAnalytics';
import LlmConsumptionPanel from './LlmConsumptionPanel';
import ToolUsagePanel from './ToolUsagePanel';

export default function AnalyticsTab() {
  const { timeRange } = useAdminTimeRange();
  const results = useQueries({
    queries: [usageAnalyticsQuery(timeRange), llmConsumptionQuery(timeRange), toolUsageQuery(timeRange)],
  });
  const settled = results.every((result) => !result.isPending);
  // Hold the panels back only for the first load. A later range change is
  // the admin's own click, and each panel shows its own spinner for it.
  const [revealed, setRevealed] = useState(settled);
  if (settled && !revealed) {
    setRevealed(true);
  }

  if (!revealed) {
    return (
      <div className="flex justify-center py-8" role="status" aria-label="Loading analytics">
        <div className="pierre-spinner w-8 h-8"></div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <UsageAnalytics />
      <LlmConsumptionPanel />
      <ToolUsagePanel />
    </div>
  );
}
