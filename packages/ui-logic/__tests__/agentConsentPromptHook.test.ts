// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared one-time agent consent question — the read, the answer, and the rereads after it
// ABOUTME: Answering rereads the question and both consent classes, so the privacy switches show what it gave (carnet#1544)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { AgentConsentPromptAnswer, AgentConsentPromptState } from '@pierre/shared-types';
import { createAgentConsentPromptHooks } from '../src/agentConsentPromptHook';

const getAgentConsentPrompt = vi.fn<() => Promise<AgentConsentPromptState>>();
const answerAgentConsentPrompt = vi.fn<(answer: AgentConsentPromptAnswer) => Promise<AgentConsentPromptState>>();
const { useAgentConsentPrompt, useAnswerAgentConsentPrompt } = createAgentConsentPromptHooks({
  getAgentConsentPrompt,
  answerAgentConsentPrompt,
});

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const invalidate = vi.spyOn(client, 'invalidateQueries');
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { invalidate, wrapper };
}

describe('createAgentConsentPromptHooks', () => {
  beforeEach(() => {
    getAgentConsentPrompt.mockReset();
    answerAgentConsentPrompt.mockReset();
  });

  it('reads whether the question is due', async () => {
    getAgentConsentPrompt.mockResolvedValue({ due: true, memory: false, medical: false });
    const { wrapper } = setup();
    const { result } = renderHook(() => useAgentConsentPrompt(), { wrapper });
    await waitFor(() => expect(result.current.data).toEqual({ due: true, memory: false, medical: false }));
  });

  it('does not read it while disabled', () => {
    const { wrapper } = setup();
    renderHook(() => useAgentConsentPrompt(false), { wrapper });
    expect(getAgentConsentPrompt).not.toHaveBeenCalled();
  });

  it('answers and rereads the question and both consent classes', async () => {
    answerAgentConsentPrompt.mockResolvedValue({ due: false, memory: true, medical: false });
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => useAnswerAgentConsentPrompt(), { wrapper });

    await act(() => result.current.mutateAsync({ memory: true, medical: false }));

    expect(answerAgentConsentPrompt).toHaveBeenCalledWith({ memory: true, medical: false });
    expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual([
      ['agent-consent-prompt'],
      ['providers-ai-consent', 'sensitive_t3'],
      ['providers-ai-consent', 'sensitive_t2'],
    ]);
  });

  it('rereads after a refused answer too', async () => {
    answerAgentConsentPrompt.mockRejectedValue(new Error('refused'));
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => useAnswerAgentConsentPrompt(), { wrapper });

    await act(async () => {
      await expect(result.current.mutateAsync({ dismissed: true })).rejects.toThrow('refused');
    });
    expect(invalidate).toHaveBeenCalledTimes(3);
  });
});
