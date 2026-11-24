// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the "Who read your data" hooks — the feed read, the shared Connected-apps revoke, and the app rows grouped per client
// ABOUTME: Revoke revokes every grant of a client and rereads both keys, even after a failure; only a held Connected app gets one (carnet#1543)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { AccessFeed, AccessFeedAppRead, OAuthGrant } from '@pierre/shared-types';
import {
  createAccessFeedHook,
  createConnectedAppsHook,
  createRevokeConnectedAppsHook,
  groupAccessFeedApiKeyDays,
  groupAccessFeedApps,
  revokedInKeySettings,
} from '../src/accessFeedHook';

const getAccessFeed = vi.fn<() => Promise<AccessFeed>>();
const listConnectedApps = vi.fn<() => Promise<OAuthGrant[]>>();
const revokeConnectedApp = vi.fn<(id: string) => Promise<void>>();
const useAccessFeed = createAccessFeedHook({ getAccessFeed });
const useConnectedApps = createConnectedAppsHook({ listConnectedApps });
const useRevokeConnectedApps = createRevokeConnectedAppsHook({ revokeConnectedApp });

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const invalidate = vi.spyOn(client, 'invalidateQueries');
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { invalidate, wrapper };
}

function read(overrides: Partial<AccessFeedAppRead> = {}): AccessFeedAppRead {
  return {
    read_at: '2026-11-20T10:00:00Z',
    kind: 'tool_call',
    tool: 'get_activities',
    tools: ['get_activities'],
    client_id: 'client-claude',
    client_name: 'Claude',
    client_host: 'claude.ai',
    credential: 'oauth_grant',
    transport: 'mcp_http',
    support_read: false,
    ...overrides,
  };
}

function grant(id: string, clientId: string, scope = 'fitness:read'): OAuthGrant {
  return {
    id,
    client_id: clientId,
    client_name: 'Claude',
    client_host: 'claude.ai',
    scope,
    granted_at: '2026-11-01T10:00:00Z',
    preset: null,
    max_tier: null,
    expires_at: null,
  };
}

const FEED: AccessFeed = { app_reads: [read()], coach_days: [], peer_days: [], api_key_days: [], days: 30 };

describe('createAccessFeedHook', () => {
  beforeEach(() => getAccessFeed.mockReset());

  it('reads the feed under its shared key', async () => {
    getAccessFeed.mockResolvedValue(FEED);
    const { wrapper } = setup();
    const { result } = renderHook(() => useAccessFeed(), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual(FEED);
  });

  it('reads nothing while disabled', () => {
    const { wrapper } = setup();
    renderHook(() => useAccessFeed(false), { wrapper });
    expect(getAccessFeed).not.toHaveBeenCalled();
  });
});

describe('createConnectedAppsHook', () => {
  it('lists the approved apps', async () => {
    listConnectedApps.mockResolvedValue([grant('g1', 'client-claude')]);
    const { wrapper } = setup();
    const { result } = renderHook(() => useConnectedApps(), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.map((g) => g.id)).toEqual(['g1']);
  });
});

describe('createRevokeConnectedAppsHook', () => {
  beforeEach(() => revokeConnectedApp.mockReset());

  it('revokes every grant it is given and rereads the Connected-apps list and the feed', async () => {
    revokeConnectedApp.mockResolvedValue(undefined);
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => useRevokeConnectedApps(), { wrapper });

    await act(() => result.current.mutateAsync(['g1', 'g2']));

    expect(revokeConnectedApp.mock.calls.map(([id]) => id)).toEqual(['g1', 'g2']);
    expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual([
      ['oauth', 'connected-apps'],
      ['access-feed'],
    ]);
  });

  it('rereads both after a refused revoke too, since part of it may have gone through', async () => {
    revokeConnectedApp.mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error('refused'));
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => useRevokeConnectedApps(), { wrapper });

    act(() => result.current.mutate(['g1', 'g2']));

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(invalidate).toHaveBeenCalledTimes(2);
  });
});

describe('groupAccessFeedApps', () => {
  it('groups a client\'s reads and offers Revoke over every grant it holds, one per scope', () => {
    const groups = groupAccessFeedApps(
      [read(), read({ tool: 'get_athlete', read_at: '2026-11-19T10:00:00Z' })],
      [grant('g1', 'client-claude'), grant('g2', 'client-claude', 'fitness:write'), grant('g3', 'other')],
    );

    expect(groups).toHaveLength(1);
    expect(groups[0]).toMatchObject({ kind: 'app', name: 'Claude', host: 'claude.ai', revokeGrantIds: ['g1', 'g2'] });
    expect(groups[0].reads.map((r) => r.tool)).toEqual(['get_activities', 'get_athlete']);
  });

  it('keeps clients apart, in the order of their newest read', () => {
    const groups = groupAccessFeedApps(
      [read({ client_id: 'b', client_name: 'B' }), read({ client_id: 'a', client_name: 'A' }), read({ client_id: 'b', client_name: 'B' })],
      [],
    );
    expect(groups.map((g) => g.name)).toEqual(['B', 'A']);
    expect(groups[0].reads).toHaveLength(2);
  });

  it('offers no Revoke to a Connected app the athlete no longer holds a grant of', () => {
    const [group] = groupAccessFeedApps([read()], []);
    expect(group.revokeGrantIds).toEqual([]);
  });

  it('offers no Revoke to an API key, an MCP token or an A2A client, even one sharing a client id', () => {
    const grants = [grant('g1', 'client-claude')];
    for (const credential of ['api_key', 'mcp_token', 'a2a_client_credentials'] as const) {
      const [group] = groupAccessFeedApps([read({ credential })], grants);
      expect(group.revokeGrantIds, credential).toEqual([]);
    }
  });

  it('puts support\'s reads in one group that names no client and has no Revoke', () => {
    const groups = groupAccessFeedApps(
      [read({ support_read: true, client_name: 'Admin console' }), read()],
      [grant('g1', 'client-claude')],
    );

    expect(groups.map((g) => g.kind)).toEqual(['support', 'app']);
    expect(groups[0]).toMatchObject({ name: null, host: null, revokeGrantIds: [] });
  });

  it('names an app by its grant when the read carries no name, else by its client id', () => {
    const withGrant = groupAccessFeedApps([read({ client_name: null, client_host: null })], [grant('g1', 'client-claude')]);
    expect(withGrant[0]).toMatchObject({ name: 'Claude', host: 'claude.ai' });

    const bare = groupAccessFeedApps([read({ client_name: null, client_host: null, credential: 'api_key' })], []);
    expect(bare[0]).toMatchObject({ name: 'client-claude', host: null });

    const anonymous = groupAccessFeedApps([read({ client_id: null, client_name: null, credential: 'api_key' })], []);
    expect(anonymous[0].name).toBeNull();
  });
});

describe('groupAccessFeedApps over agent turns', () => {
  it('keeps an app\'s agent turns with its tool calls, in the order the server sent them', () => {
    const turn = read({ kind: 'agent_turn', tool: null, tools: ['get_activities', 'get_athlete'], read_at: '2026-11-20T11:00:00Z' });
    const groups = groupAccessFeedApps([turn, read()], []);
    expect(groups).toHaveLength(1);
    expect(groups[0].reads.map((r) => r.kind)).toEqual(['agent_turn', 'tool_call']);
  });
});

describe('groupAccessFeedApiKeyDays', () => {
  const day = (key_id: string, date: string) => ({
    day: date,
    key_id,
    key_name: key_id === 'k1' ? 'Export script' : 'Garage sync',
    key_prefix: `pk_${key_id}`,
    requests: 3,
  });

  it('groups each key\'s days under its name and prefix, in the order of its newest day', () => {
    const groups = groupAccessFeedApiKeyDays([day('k2', '2026-11-21'), day('k1', '2026-11-20'), day('k2', '2026-11-19')]);
    expect(groups.map((g) => [g.keyId, g.name, g.prefix])).toEqual([
      ['k2', 'Garage sync', 'pk_k2'],
      ['k1', 'Export script', 'pk_k1'],
    ]);
    expect(groups[0].days.map((d) => d.day)).toEqual(['2026-11-21', '2026-11-19']);
  });

  it('is empty for no days', () => {
    expect(groupAccessFeedApiKeyDays([])).toEqual([]);
  });
});

describe('revokedInKeySettings', () => {
  it('points an API key and an MCP token to their settings, nothing else', () => {
    expect(revokedInKeySettings('api_key')).toBe(true);
    expect(revokedInKeySettings('mcp_token')).toBe(true);
    expect(revokedInKeySettings('oauth_grant')).toBe(false);
    expect(revokedInKeySettings('a2a_client_credentials')).toBe(false);
    expect(revokedInKeySettings(null)).toBe(false);
  });
});
