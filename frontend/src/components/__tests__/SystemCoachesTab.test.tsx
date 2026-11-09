// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Agents edit form sends only the fields a system agent stores
// ABOUTME: The structured sections an agent carries are shown elsewhere, never echoed back for the server to refuse

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Agent } from '../../types/api';
import SystemCoachesTab from '../SystemCoachesTab';

const AGENT: Agent = {
  id: 'agent-1',
  title: 'Marathon Agent',
  description: 'Paces marathons',
  system_prompt: 'You answer marathon questions.',
  category: 'training',
  tags: ['marathon'],
  token_count: 10,
  is_favorite: false,
  use_count: 0,
  last_used_at: null,
  created_at: '2026-11-01T00:00:00Z',
  updated_at: '2026-11-01T00:00:00Z',
  is_system: true,
  visibility: 'tenant',
  purpose: 'Pace a marathon',
  when_to_use: 'Before a race',
  instructions: 'Answer briefly',
  example_inputs: 'How fast?',
  example_outputs: '5:00/km',
  success_criteria: 'The athlete finishes',
};

vi.mock('../../services/api', () => ({
  adminApi: {
    getSystemCoaches: vi.fn(),
    getAllUsers: vi.fn().mockResolvedValue([]),
    getCoachAssignments: vi.fn().mockResolvedValue({ assignments: [] }),
    createSystemCoach: vi.fn(),
    updateSystemCoach: vi.fn(),
  },
}));

const { adminApi } = await import('../../services/api');

function renderTab() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <SystemCoachesTab />
    </QueryClientProvider>,
  );
}

describe('SystemCoachesTab edit form', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(adminApi.getSystemCoaches).mockResolvedValue({
      agents: [AGENT],
      total: 1,
      metadata: { timestamp: '', api_version: '' },
    });
    vi.mocked(adminApi.updateSystemCoach).mockResolvedValue(AGENT);
  });

  it('saves only what a system agent stores', async () => {
    renderTab();
    fireEvent.click(await screen.findByText('Marathon Agent'));
    fireEvent.click(await screen.findByRole('button', { name: /Edit/ }));
    fireEvent.click(await screen.findByRole('button', { name: 'Save Changes' }));

    await waitFor(() => expect(adminApi.updateSystemCoach).toHaveBeenCalledTimes(1));
    const [id, body] = vi.mocked(adminApi.updateSystemCoach).mock.calls[0];
    expect(id).toBe('agent-1');
    expect(Object.keys(body).sort()).toEqual(
      ['category', 'description', 'system_prompt', 'tags', 'title'],
    );
  });
});
