// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared AI-consent mutation — one call either way, and both provider reads reread after it
// ABOUTME: A failed change rereads them too, so every consent control shows the server's answer

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { createAiConsentHook } from '../src/aiConsentHook';

const grantAiConsent = vi.fn<(provider: string) => Promise<void>>();
const withdrawAiConsent = vi.fn<(provider: string) => Promise<void>>();
const useAiConsent = createAiConsentHook({ grantAiConsent, withdrawAiConsent });

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const invalidate = vi.spyOn(client, 'invalidateQueries');
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { invalidate, wrapper };
}

describe('createAiConsentHook', () => {
  beforeEach(() => {
    grantAiConsent.mockReset();
    withdrawAiConsent.mockReset();
  });

  it('withdraws with one call and rereads both provider reads', async () => {
    withdrawAiConsent.mockResolvedValue(undefined);
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => useAiConsent(), { wrapper });

    await act(() => result.current.mutateAsync({ provider: 'whoop', allow: false }));

    expect(withdrawAiConsent).toHaveBeenCalledWith('whoop');
    expect(grantAiConsent).not.toHaveBeenCalled();
    expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual([
      ['provider-connections'],
      ['providers-status'],
    ]);
  });

  it('gives consent back with the other call', async () => {
    grantAiConsent.mockResolvedValue(undefined);
    const { wrapper } = setup();
    const { result } = renderHook(() => useAiConsent(), { wrapper });

    await act(() => result.current.mutateAsync({ provider: 'whoop', allow: true }));

    expect(grantAiConsent).toHaveBeenCalledWith('whoop');
    expect(withdrawAiConsent).not.toHaveBeenCalled();
  });

  it('rereads both provider reads after a refused change too', async () => {
    withdrawAiConsent.mockRejectedValue(new Error('refused'));
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => useAiConsent(), { wrapper });

    act(() => result.current.mutate({ provider: 'whoop', allow: false }));

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(invalidate).toHaveBeenCalledTimes(2);
  });
});
