// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the agent edit sheet's Store group — the submit call, the confirmation, and the server's refusal
// ABOUTME: The coaches API is mocked at the services barrel; assertions read the call and the rendered text

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import CoachStoreSubmit from '../CoachStoreSubmit';

const submitToStore = vi.fn();

vi.mock('../../../services/api', () => ({
  coachesApi: {
    submitToStore: (...a: unknown[]) => submitToStore(...a),
  },
}));

function renderSubmit() {
  const queryClient = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <CoachStoreSubmit agentId="coach-hills" />
    </QueryClientProvider>,
  );
}

describe('CoachStoreSubmit', () => {
  beforeEach(() => {
    submitToStore.mockReset();
  });

  it('submits the agent and says it waits for an admin', async () => {
    submitToStore.mockResolvedValue({
      agent_id: 'coach-hills',
      publish_status: 'pending_review',
      review_submitted_at: '2026-09-28T10:00:00Z',
    });
    const user = userEvent.setup();
    renderSubmit();

    await user.click(screen.getByTestId('agent-store-submit-button'));

    expect(submitToStore).toHaveBeenCalledWith('coach-hills');
    expect(await screen.findByTestId('agent-store-submitted')).toHaveTextContent(
      'Submitted — an admin will review it before it appears in the Store.',
    );
    expect(screen.queryByTestId('agent-store-submit-button')).not.toBeInTheDocument();
  });

  it('says why the server refused', async () => {
    submitToStore.mockRejectedValue(new Error('network down'));
    const user = userEvent.setup();
    renderSubmit();

    await user.click(screen.getByTestId('agent-store-submit-button'));

    expect(await screen.findByRole('alert')).toBeInTheDocument();
    expect(screen.getByTestId('agent-store-submit-button')).toBeEnabled();
  });
});
