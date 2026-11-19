// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Agents edit form sends only the fields a system agent stores
// ABOUTME: The structured sections an agent carries are shown elsewhere, never echoed back for the server to refuse

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Agent, User } from '../../types/api';
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

  it('closes the assign-users dialog on Escape and hands focus back to its button', async () => {
    renderTab();
    fireEvent.click(await screen.findByText('Marathon Agent'));
    const assign = await screen.findByRole('button', { name: /Assign Users/ });
    assign.focus();
    fireEvent.click(assign);

    const dialog = screen.getByRole('dialog', { name: 'Assign Users to Agent' });
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
    fireEvent.keyDown(document, { key: 'Escape' });

    expect(screen.queryByRole('dialog', { name: 'Assign Users to Agent' })).not.toBeInTheDocument();
    expect(assign).toHaveFocus();
  });
});

function user(overrides: Partial<User>): User {
  return {
    id: 'user',
    email: 'user@example.com',
    is_admin: false,
    role: 'user',
    tier: 'starter',
    created_at: '2026-11-01T00:00:00Z',
    ...overrides,
  };
}

describe('SystemCoachesTab badges', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('tags a git-synced agent as a success and any other source in the muted tone', async () => {
    vi.mocked(adminApi.getSystemCoaches).mockResolvedValue({
      agents: [
        { ...AGENT, id: 'agent-git', title: 'Synced Agent', source: 'contremaitre' },
        { ...AGENT, id: 'agent-seed', title: 'Seeded Agent', source: 'seed' },
      ],
      total: 2,
      metadata: { timestamp: '', api_version: '' },
    });
    renderTab();

    expect(await screen.findByText('git')).toHaveClass('badge-success');
    expect(screen.getByText('seed')).toHaveClass('badge-secondary');
  });

  it('draws an active user as a success and any other status in the muted tone in the assign dialog', async () => {
    vi.mocked(adminApi.getSystemCoaches).mockResolvedValue({
      agents: [AGENT],
      total: 1,
      metadata: { timestamp: '', api_version: '' },
    });
    vi.mocked(adminApi.getAllUsers).mockResolvedValue([
      user({ id: 'u1', email: 'active@example.com', user_status: 'active' }),
      user({ id: 'u2', email: 'suspended@example.com', user_status: 'suspended' }),
    ]);
    renderTab();
    fireEvent.click(await screen.findByText('Marathon Agent'));
    fireEvent.click(await screen.findByRole('button', { name: /Assign Users/ }));

    expect(await screen.findByText('active')).toHaveClass('badge-success');
    expect(screen.getByText('suspended')).toHaveClass('badge-secondary');
  });
});
