// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the grant history and standing-policy hooks — what each reads, by page, and the removal's reread
// ABOUTME: A removal rereads the policies even after a failure; an unnamed grant reads no receipts (carnet#1544)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { OAuthGrantReceipt, OAuthGrantReceiptPage, OAuthStandingPolicy } from '@pierre/shared-types';
import { QUERY_KEYS } from '@pierre/shared-constants';
import {
  createConnectedAppReceiptsHook,
  createGrantReceiptsHook,
  createRemoveStandingPolicyHook,
  createStandingPoliciesHook,
  GRANT_RECEIPT_PAGE,
} from '../src/grantHistoryHooks';

const listConnectedAppReceipts = vi.fn<(id: string, page?: OAuthGrantReceiptPage) => Promise<OAuthGrantReceipt[]>>();
const listGrantReceipts = vi.fn<(page?: OAuthGrantReceiptPage) => Promise<OAuthGrantReceipt[]>>();
const listStandingPolicies = vi.fn<() => Promise<OAuthStandingPolicy[]>>();
const removeStandingPolicy = vi.fn<(id: string) => Promise<void>>();
const useConnectedAppReceipts = createConnectedAppReceiptsHook({ listConnectedAppReceipts });
const useGrantReceipts = createGrantReceiptsHook({ listGrantReceipts });
const useStandingPolicies = createStandingPoliciesHook({ listStandingPolicies });
const useRemoveStandingPolicy = createRemoveStandingPolicyHook({ removeStandingPolicy });

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const invalidate = vi.spyOn(client, 'invalidateQueries');
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { invalidate, wrapper };
}

const receipt: OAuthGrantReceipt = {
  id: 'r1',
  client_grant_id: 'g1',
  event: 'revoke',
  actor: 'user',
  at: '2026-11-24T10:00:00Z',
  payload: {},
  payload_sha256: 'ab',
  jws: null,
  logged_at: null,
};

const policy: OAuthStandingPolicy = {
  id: 'p1',
  client_id: 'c1',
  client_name: null,
  client_host: 'claude.ai',
  preset: 'sleep_coach',
  created_at: '2026-11-24T10:00:00Z',
};

beforeEach(() => {
  vi.clearAllMocks();
});

describe('grant history hooks', () => {
  it('reads a named grant\'s receipts, and none before a grant is named', async () => {
    listConnectedAppReceipts.mockResolvedValue([receipt]);
    const { wrapper } = setup();
    const idle = renderHook(() => useConnectedAppReceipts(null), { wrapper });
    expect(idle.result.current.fetchStatus).toBe('idle');
    expect(listConnectedAppReceipts).not.toHaveBeenCalled();

    const named = renderHook(() => useConnectedAppReceipts('g1'), { wrapper });
    await waitFor(() => expect(named.result.current.data).toEqual([receipt]));
    expect(listConnectedAppReceipts).toHaveBeenCalledWith('g1');
  });

  it('reads every receipt a page at a time', async () => {
    listGrantReceipts.mockResolvedValue([receipt]);
    const { wrapper } = setup();
    const { result } = renderHook(() => useGrantReceipts(2), { wrapper });
    await waitFor(() => expect(result.current.data).toEqual([receipt]));
    expect(listGrantReceipts).toHaveBeenCalledWith({ limit: GRANT_RECEIPT_PAGE, offset: 2 * GRANT_RECEIPT_PAGE });
  });

  it('lists the policies and rereads them after a removal, even a failed one', async () => {
    listStandingPolicies.mockResolvedValue([policy]);
    const { invalidate, wrapper } = setup();
    const listed = renderHook(() => useStandingPolicies(), { wrapper });
    await waitFor(() => expect(listed.result.current.data).toEqual([policy]));

    removeStandingPolicy.mockRejectedValueOnce(new Error('offline'));
    const removal = renderHook(() => useRemoveStandingPolicy(), { wrapper });
    await act(async () => {
      await removal.result.current.mutateAsync('p1').catch(() => undefined);
    });
    expect(removeStandingPolicy).toHaveBeenCalledWith('p1');
    expect(invalidate).toHaveBeenCalledWith({ queryKey: QUERY_KEYS.oauth.standingPolicies() });
  });
});
