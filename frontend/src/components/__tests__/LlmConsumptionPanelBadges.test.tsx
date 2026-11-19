// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders LlmConsumptionPanel to pin each breakdown row's call-type pill to the badge tone table
// ABOUTME: Chat is primary, insight the solid accent, anything else neutral

import React from 'react';
import { render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { describe, expect, it, vi } from 'vitest';
import type { LlmConsumptionResponse } from '../../services/api/usage';
import { ThemeProvider } from '../../hooks/useTheme';

const consumption: LlmConsumptionResponse = {
  scope: { kind: 'all_tenants' },
  summary: { total_tokens: 3500, total_calls: 18, estimated_cost_usd: 0.6 },
  breakdown: [
    { provider: 'gemini', model: 'gemini-2.0-flash', call_type: 'chat', total_tokens: 1000, calls: 5, cost_usd: 0.1 },
    { provider: 'gemini', model: 'gemini-2.5-pro', call_type: 'insight', total_tokens: 2000, calls: 3, cost_usd: 0.5 },
    { provider: 'direct', model: 'get_activities', call_type: 'mcp_tool', total_tokens: 500, calls: 10, cost_usd: 0 },
  ],
  daily_series: [],
};

// The network seam: the panel reads its breakdown from the admin usage API,
// so the fixture stands in for the server. The real panel, query and theme run.
vi.mock('../../services/api', () => ({
  adminUsageApi: { getAdminLlmConsumption: () => Promise.resolve(consumption) },
}));

// `useAdminScope` reads the viewer's role from the auth context. A plain admin
// sends no tenant scope, so the role is all this panel needs from it.
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { role: 'admin' } }),
}));

import LlmConsumptionPanel from '../LlmConsumptionPanel';

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <ThemeProvider>
        <LlmConsumptionPanel />
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

describe('LlmConsumptionPanel call-type pills', () => {
  it('draws each call type in its badge tone', async () => {
    renderPanel();

    const pill = async (text: string) =>
      (await screen.findAllByText(text)).find((element) => element.classList.contains('rounded-full'));
    expect(await pill('chat')).toHaveClass('badge-primary');
    expect(await pill('insight')).toHaveClass('badge-accent');
    expect(await pill('mcp tool')).toHaveClass('badge-neutral');
  });
});
