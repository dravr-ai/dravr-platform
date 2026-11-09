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
    schema_version: 3,
    verification: { runtime_judge: true },
    compaction: {
      window_tokens: 100_000,
      warn_threshold: 0.6,
      emergency_threshold: 0.9,
      summarize_oldest_n: 5,
      sliding_drop_n: 3,
      max_messages: 40,
    },
    memory: { dedup_candidate_limit: 50 },
    guardrails: {
      blocked_topics: ['self-harm'],
      locales: {
        en: {
          disclaimer_triggers: ['injury'],
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
    in_force: {
      instance_id: 'rev-1:abc',
      loaded_at: '2026-04-13T18:00:00Z',
      in_sync: true,
    },
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
    vi.mocked(adminApi.getHarnessConfig).mockReset();
    vi.mocked(adminApi.putHarnessConfig).mockReset();
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
    // window_tokens, warn, emergency, summarize_n, sliding_drop_n, max_messages, dedup
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
    fireEvent.change(screen.getAllByRole('spinbutton')[0], { target: { value: '7777' } });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalledTimes(1);
    });
    expect(vi.mocked(adminApi.putHarnessConfig).mock.calls[0][1]).toBe(read);

    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce(sampleResponse());
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    fireEvent.change(screen.getAllByRole('spinbutton')[0], { target: { value: '8888' } });
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
    fireEvent.change(screen.getAllByRole('spinbutton')[0], { target: { value: '7777' } });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalled();
    });
    expect(vi.mocked(adminApi.putHarnessConfig).mock.calls[0][1]).toBeNull();
  });

  const stored = {
    ...sampleResponse(),
    updated_at: '2026-04-13T19:00:00Z',
    config: {
      ...sampleDoc(),
      compaction: { ...sampleDoc().compaction, sliding_drop_n: 9 },
    },
  };

  async function conflictOnSave() {
    vi.mocked(adminApi.getHarnessConfig)
      .mockResolvedValueOnce(sampleResponse())
      .mockResolvedValueOnce(stored);
    vi.mocked(adminApi.putHarnessConfig).mockRejectedValueOnce({
      response: { data: { code: 'harness_config_stale', message: 'It changed.' } },
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    fireEvent.change(screen.getAllByRole('spinbutton')[0], { target: { value: '7777' } });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await screen.findByRole('alert');
  }

  it('a stale save keeps the draft, names both sides, and cannot be saved again', async () => {
    await conflictOnSave();

    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('It changed.');
    expect(alert).toHaveTextContent('your edits are still in this form');
    expect(alert).toHaveTextContent('compaction.sliding_drop_n');
    expect(alert).toHaveTextContent('compaction.window_tokens');
    expect(screen.getByDisplayValue(/7777/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /save changes/i })).toBeDisabled();
    expect(adminApi.putHarnessConfig).toHaveBeenCalledTimes(1);
  });

  it('applying the edits keeps the other save and names the newer version', async () => {
    await conflictOnSave();

    fireEvent.click(screen.getByRole('button', { name: /apply my edits/i }));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(adminApi.putHarnessConfig).toHaveBeenCalledTimes(1);

    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce(sampleResponse());
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalledTimes(2);
    });
    const [payload, token] = vi.mocked(adminApi.putHarnessConfig).mock.calls[1];
    expect(token).toBe('2026-04-13T19:00:00Z');
    expect(payload.compaction.window_tokens).toBe(7777);
    expect(payload.compaction.sliding_drop_n).toBe(9);
  });

  it('discarding the edits shows the stored version', async () => {
    await conflictOnSave();

    fireEvent.click(screen.getByRole('button', { name: /discard my edits/i }));

    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.queryByDisplayValue(/7777/)).not.toBeInTheDocument();
    expect(screen.getByDisplayValue('9')).toBeInTheDocument();
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

    expect(await screen.findByText(/stored version could not be read/)).toBeInTheDocument();
    expect(screen.getByDisplayValue(/7777/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /save changes/i })).toBeDisabled();
  });

  it('edits the triggers but not the disclaimer wording, which the catalogue owns', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    expect(await screen.findByLabelText(/Triggers \(en\)/)).toHaveValue('injury');
    expect(screen.queryByLabelText(/Disclaimer text/)).not.toBeInTheDocument();
    expect(screen.getByText(/comes from the string catalogue/)).toBeInTheDocument();
  });

  it('save stays off until something changed', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByDisplayValue(/100000/)).toBeInTheDocument();
    });
    const save = screen.getByRole('button', { name: /save changes/i });
    expect(save).toBeDisabled();
    fireEvent.change(screen.getAllByRole('spinbutton')[0], { target: { value: '7777' } });
    expect(save).not.toBeDisabled();
  });

  it('shows and saves max messages and the memory dedup limit', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    fireEvent.change(await screen.findByLabelText('Max messages'), { target: { value: '60' } });
    fireEvent.change(screen.getByLabelText('Memory dedup candidates'), { target: { value: '25' } });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalled();
    });
    const [payload] = vi.mocked(adminApi.putHarnessConfig).mock.calls[0];
    expect(payload.compaction.max_messages).toBe(60);
    expect(payload.memory.dedup_candidate_limit).toBe(25);
  });

  it('holds the same rules the server does for compaction', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    const summarize = await screen.findByLabelText('Summarize oldest N');
    fireEvent.change(summarize, { target: { value: '39' } });
    expect(screen.getByText(/plus 2 must not exceed max messages/)).toBeInTheDocument();
    fireEvent.change(summarize, { target: { value: '1' } });
    expect(screen.getByText(/must be at least 2/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /save changes/i })).toBeDisabled();
  });

  it('a cleared number is invalid rather than a saved 0', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    const maxMessages = await screen.findByLabelText('Max messages');
    fireEvent.change(maxMessages, { target: { value: '' } });
    expect(maxMessages).toHaveValue(null);
    expect(screen.getByText(/must be above 0/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /save changes/i })).toBeDisabled();
  });

  it('says a blocked phrase replaces the whole reply, a debunking one included', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await screen.findByLabelText(/Blocked topics/);
    expect(screen.getByText(/is replaced whole by a refusal/)).toBeInTheDocument();
    expect(screen.getByText(/a reply quoting the phrase to debunk it is replaced too/)).toBeInTheDocument();
  });

  it('a list field keeps what is being typed, and saves it as a list', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putHarnessConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    const topics = await screen.findByLabelText(/Blocked topics/);
    fireEvent.change(topics, { target: { value: 'self-harm, weight ' } });
    expect(topics).toHaveValue('self-harm, weight ');
    fireEvent.change(topics, { target: { value: 'self-harm, weight cutting' } });
    expect(topics).toHaveValue('self-harm, weight cutting');
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putHarnessConfig).toHaveBeenCalled();
    });
    const [payload] = vi.mocked(adminApi.putHarnessConfig).mock.calls[0];
    expect(payload.guardrails.blocked_topics).toEqual(['self-harm', 'weight cutting']);
  });

  it('says what the warn threshold works out to, and steps finely enough to hold it', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce({
      ...sampleResponse(),
      config: {
        ...sampleDoc(),
        compaction: { ...sampleDoc().compaction, window_tokens: 1_000_000, warn_threshold: 0.0896, emergency_threshold: 0.1216 },
      },
    });
    renderTab();
    const warn = await screen.findByLabelText('Warn threshold');
    expect(warn).toHaveAttribute('step', '0.0001');
    expect(screen.getByText(/89,600 tokens/)).toBeInTheDocument();
    expect(screen.getByText(/121,600 tokens/)).toBeInTheDocument();
  });

  it('says whether the instance that answered runs what is stored', async () => {
    vi.mocked(adminApi.getHarnessConfig).mockResolvedValueOnce({
      ...sampleResponse(),
      in_force: {
        instance_id: 'rev-2:def',
        loaded_at: '2026-04-13T18:00:00Z',
        in_sync: false,
        load_failure: 'the row could not be read at boot',
      },
    });
    renderTab();
    const line = await screen.findByTestId('harness-in-force');
    expect(line).toHaveTextContent('is not running these values yet');
    expect(line).toHaveTextContent('rev-2:def');
    expect(line).toHaveTextContent('the row could not be read at boot');
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
