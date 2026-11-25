// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the routes and pages a Connected app's grant receipts and the standing policies are read and removed on
// ABOUTME: Red if a path drifts from the server's /api/me routes or a page's wire names (`limit`, `offset`) change

import { describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createOAuthApi } from '../src/domains/oauth';

const receipt = {
  id: 'r1',
  client_grant_id: 'g1',
  event: 'issue',
  actor: 'auto',
  at: '2026-11-24T10:00:00Z',
  payload: { receipt_version: 0 },
  payload_sha256: 'ab',
  jws: null,
  logged_at: null,
};

describe('the grant history and standing policies (carnet#1544)', () => {
  it('reads one grant\'s receipts and every receipt by page', async () => {
    const get = vi.fn().mockResolvedValue({ data: { receipts: [receipt] } });
    const api = createOAuthApi({ get } as unknown as AxiosInstance);

    await expect(api.listConnectedAppReceipts('g1', { limit: 10 })).resolves.toEqual([receipt]);
    expect(get).toHaveBeenLastCalledWith('/api/me/oauth-grants/g1/receipts', { params: { limit: 10 } });

    await expect(api.listGrantReceipts({ limit: 50, offset: 100 })).resolves.toEqual([receipt]);
    expect(get).toHaveBeenLastCalledWith('/api/me/oauth-grant-receipts', { params: { limit: 50, offset: 100 } });

    await api.listGrantReceipts();
    expect(get).toHaveBeenLastCalledWith('/api/me/oauth-grant-receipts', { params: {} });
  });

  it('lists the policies and removes one by id', async () => {
    const policy = {
      id: 'p1',
      client_id: 'c1',
      client_name: 'Claude',
      client_host: 'claude.ai',
      preset: 'training_coach',
      created_at: '2026-11-24T10:00:00Z',
    };
    const get = vi.fn().mockResolvedValue({ data: { policies: [policy] } });
    const del = vi.fn().mockResolvedValue({ status: 204 });
    const api = createOAuthApi({ get, delete: del } as unknown as AxiosInstance);

    await expect(api.listStandingPolicies()).resolves.toEqual([policy]);
    expect(get).toHaveBeenLastCalledWith('/api/me/oauth-standing-policies');

    await api.removeStandingPolicy('p1');
    expect(del).toHaveBeenLastCalledWith('/api/me/oauth-standing-policies/p1');
  });
});
