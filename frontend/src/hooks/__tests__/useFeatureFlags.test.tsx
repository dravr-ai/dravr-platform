// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the useFeatureFlags hook
// ABOUTME: Covers fallback defaults, server merge, and FEATURE_KEYS constants

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { useFeatureFlags, FEATURE_KEYS } from '../useFeatureFlags';

vi.mock('../../services/api/featureFlags', () => ({
  featureFlagsApi: {
    getMyFeatures: vi.fn(),
  },
}));

import { featureFlagsApi } from '../../services/api/featureFlags';
const getMyFeaturesMock = vi.mocked(featureFlagsApi.getMyFeatures);

function createWrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

describe('useFeatureFlags', () => {
  beforeEach(() => {
    getMyFeaturesMock.mockReset();
  });

  it('returns compile-default flags before the request resolves', () => {
    // Never-resolving promise so the hook stays in the loading state.
    getMyFeaturesMock.mockImplementation(() => new Promise(() => {}));
    const { result } = renderHook(() => useFeatureFlags(), { wrapper: createWrapper() });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.flags.api_tokens).toBe(false);
    expect(result.current.flags.billing_header).toBe(false);
  });

  it('merges server flags over fallback defaults', async () => {
    getMyFeaturesMock.mockResolvedValueOnce({
      flags: { api_tokens: true, billing_header: true },
      known: [
        { key: 'api_tokens', description: 'API Tokens tab', default_enabled: false },
        { key: 'billing_header', description: 'Billing header', default_enabled: false },
      ],
    });
    const { result } = renderHook(() => useFeatureFlags(), { wrapper: createWrapper() });

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.flags.api_tokens).toBe(true);
    expect(result.current.flags.billing_header).toBe(true);
    expect(result.current.known).toHaveLength(2);
  });

  it('keeps fallback values for flags the server omits', async () => {
    // Server only returns api_tokens; billing_header should remain false from FALLBACK_FLAGS.
    getMyFeaturesMock.mockResolvedValueOnce({
      flags: { api_tokens: true },
      known: [],
    });
    const { result } = renderHook(() => useFeatureFlags(), { wrapper: createWrapper() });

    await waitFor(() => expect(result.current.flags.api_tokens).toBe(true));
    expect(result.current.flags.billing_header).toBe(false);
  });

  it('falls back to defaults when the request errors', async () => {
    getMyFeaturesMock.mockRejectedValueOnce(new Error('network down'));
    const { result } = renderHook(() => useFeatureFlags(), { wrapper: createWrapper() });

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.flags.api_tokens).toBe(false);
    expect(result.current.flags.billing_header).toBe(false);
  });

  it('exposes stable storage-string constants', () => {
    expect(FEATURE_KEYS.apiTokens).toBe('api_tokens');
    expect(FEATURE_KEYS.billingHeader).toBe('billing_header');
  });
});
