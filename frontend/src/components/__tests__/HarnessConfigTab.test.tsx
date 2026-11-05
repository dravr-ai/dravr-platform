// ABOUTME: Tests for the Phase B HarnessConfigTab admin form
// ABOUTME: Mocks the admin API and asserts hydration, validation, and save behavior
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import HarnessConfigTab from '../HarnessConfigTab';
import type { HarnessConfigDocument, HarnessConfigResponse } from '../../services/api/admin';

vi.mock('../../services/api/admin', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../services/api/admin')>()),
  adminApi: {
    getHarnessConfig: vi.fn(),
    putHarnessConfig: vi.fn(),
  },
}));

const { adminApi } = await import('../../services/api/admin');

function sampleDoc(): HarnessConfigDocument {
  return {
    schema_version: 2,
    verification: { runtime_judge: true },
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
      locales: {
        en: {
          disclaimer_triggers: ['injury'],
          disclaimer_text: 'Speak to a doctor.',
        },
      },
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
  render(
    <QueryClientProvider client={queryClient}>
      <HarnessConfigTab />
    </QueryClientProvider>,
  );
  return queryClient;
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

  it('echoes the updated_at it read verbatim, and the one a save returned next', async () => {
    const read = '2026-04-13T18:00:00.123456+00:00';
    const written = '2026-04-13T18:05:00.654321+00:00';
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce({
      ...sampleResponse(),
      updated_at: read,
    });
    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce({
      ...sampleResponse(),
      updated_at: written,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalledTimes(1);
    });
    expect(vi.mocked(adminApi.putHarnessConfig).mock.calls[0][1]).toBe(read);

    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce(sampleResponse());
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /save changes/i })).not.toBeDisabled();
    });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalledTimes(2);
    });
    expect(vi.mocked(adminApi.putHarnessConfig).mock.calls[1][1]).toBe(written);
  });

  it('sends null when it read no row', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse(false));
    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalled();
    });
    expect(vi.mocked(adminApi.putHarnessConfig).mock.calls[0][1]).toBeNull();
  });

  it('reloads and says so when the server calls the save stale', async () => {
    vi.mocked(adminApi.getHarnessConfig)
      .mockResolvedValueOnce(sampleResponse())
      .mockResolvedValueOnce({
        ...sampleResponse(),
        updated_at: '2026-04-13T19:00:00Z',
      });
    vi.mocked(adminApi.putHarnessConfig).mockRejectedValueOnce({
      response: { data: { code: 'harness_config_stale', message: 'It changed.' } },
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    expect(await screen.findByText(/It changed\..*stored configuration/)).toBeInTheDocument();
    expect(adminApi.getHarnessConfig).toHaveBeenCalledTimes(2);

    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce(sampleResponse());
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalledTimes(2);
    });
    expect(vi.mocked(adminApi.putHarnessConfig).mock.calls[1][1]).toBe('2026-04-13T19:00:00Z');
  });

  it('a stale save keeps the edits when the reload fails, and says so', async () => {
    vi.mocked(adminApi.getHarnessConfig)
      .mockResolvedValueOnce(sampleResponse())
      .mockRejectedValueOnce(new Error('unreachable'));
    vi.mocked(adminApi.putHarnessConfig).mockRejectedValueOnce({
      response: { data: { code: 'harness_config_stale', message: 'It changed.' } },
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    fireEvent.change(screen.getAllByRole('spinbutton')[0], { target: { value: '7777' } });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));

    expect(await screen.findByText(/It changed\..*edits are still here/)).toBeInTheDocument();
    expect(screen.getByDisplayValue(/7777/)).toBeInTheDocument();
  });

  it('a background refetch leaves edits and their token alone', async () => {
    const moved = { ...sampleResponse(), updated_at: '2026-04-13T19:00:00Z' };
    vi.mocked(adminApi.getHarnessConfig)
      .mockResolvedValueOnce(sampleResponse())
      .mockResolvedValueOnce(moved);
    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce(moved);
    const queryClient = renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    fireEvent.change(screen.getAllByRole('spinbutton')[0], { target: { value: '7777' } });

    await act(async () => {
      await queryClient.refetchQueries({ queryKey: ['admin', 'harness-config'] });
    });

    expect(screen.getByDisplayValue(/7777/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalled();
    });
    expect(vi.mocked(adminApi.putHarnessConfig).mock.calls[0][1]).toBe(
      sampleResponse().updated_at,
    );
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

  it('unchecking the runtime judge sends runtime_judge false', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    const judgeToggle = screen.getByRole('checkbox', {
      name: /runtime llm judge/i,
    });
    expect(judgeToggle).toBeChecked();
    fireEvent.click(judgeToggle);
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalled();
    });
    const [payload] = vi.mocked(adminApi.putHarnessConfig).mock.calls[0];
    expect(payload.verification).toEqual({ runtime_judge: false });
  });
});
