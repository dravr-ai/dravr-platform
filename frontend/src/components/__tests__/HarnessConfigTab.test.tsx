// ABOUTME: Tests for the Phase B HarnessConfigTab admin form
// ABOUTME: Mocks the admin API and asserts hydration, validation, and save behavior
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import HarnessConfigTab from '../HarnessConfigTab';
import type { HarnessConfigDocument, HarnessConfigResponse } from '../../services/api/admin';

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    getHarnessConfig: vi.fn(),
    putHarnessConfig: vi.fn(),
  },
}));

const { adminApi } = await import('../../services/api/admin');

function sampleDoc(): HarnessConfigDocument {
  return {
    schema_version: 1,
    compaction: {
      window_tokens: 100_000,
      warn_threshold: 0.6,
      emergency_threshold: 0.9,
      summarize_oldest_n: 5,
      sliding_drop_n: 3,
    },
    guardrails: {
      max_response_chars: 4_000,
      blocked_topics: ['self-harm'],
      disclaimer_triggers: ['injury'],
      disclaimer_text: 'Speak to a doctor.',
    },
  };
}

function sampleResponse(persisted: boolean = true): HarnessConfigResponse {
  return {
    config: sampleDoc(),
    source: persisted ? 'persisted' : 'default',
    updated_at: persisted ? '2026-04-13T18:00:00Z' : null,
  };
}

function renderTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <HarnessConfigTab />
    </QueryClientProvider>,
  );
}

describe('HarnessConfigTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('hydrates form fields from the persisted document', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      const windowInput = screen.getByDisplayValue(/100000/);
      expect(windowInput).toBeInTheDocument();
    });
    expect(screen.getByText(/persisted/i)).toBeInTheDocument();
  });

  it('shows the using-defaults badge when no persisted row exists', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse(false));
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/using defaults/i)).toBeInTheDocument();
    });
  });

  it('disables save when warn >= emergency', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    const inputs = screen.getAllByRole('spinbutton');
    // window_tokens, warn, emergency, summarize_n, sliding_drop_n, max_response_chars
    const warnInput = inputs[1];
    fireEvent.change(warnInput, { target: { value: '0.95' } });
    expect(screen.getByText(/warn must be strictly less than emergency/i)).toBeInTheDocument();
    const saveButton = screen.getByRole('button', { name: /save changes/i });
    expect(saveButton).toBeDisabled();
  });

  it('calls putHarnessConfig with the edited document on save', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    const inputs = screen.getAllByRole('spinbutton');
    fireEvent.change(inputs[0], { target: { value: '200000' } });
    const saveButton = screen.getByRole('button', { name: /save changes/i });
    fireEvent.click(saveButton);
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalled();
    });
    const [payload] = vi.mocked(adminApi.putHarnessConfig).mock.calls[0];
    expect(payload.compaction.window_tokens).toBe(200000);
  });

  it('reset reverts in-progress edits to the persisted document', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    const inputs = screen.getAllByRole('spinbutton');
    fireEvent.change(inputs[0], { target: { value: '7777' } });
    expect(screen.getByDisplayValue(/7777/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /reset/i }));
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
  });
});
