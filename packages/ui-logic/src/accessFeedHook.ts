// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks for the "Who read your data" feed and the Connected apps it revokes, per client's API
// ABOUTME: One Connected-apps key and one revoke for the feed and both Connected-apps screens; the app rows grouped per client (carnet#1543)

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { AccessFeedApi, OAuthApi } from '@pierre/api-client';
import type {
  AccessFeedApiKeyDay,
  AccessFeedAppRead,
  AccessFeedCredential,
  AccessFeedTransport,
  OAuthGrant,
} from '@pierre/shared-types';

/** Build `useAccessFeed` over one client's access feed API. */
export function createAccessFeedHook(api: Pick<AccessFeedApi, 'getAccessFeed'>) {
  return function useAccessFeed(enabled = true) {
    return useQuery({
      queryKey: QUERY_KEYS.accessFeed.feed(),
      queryFn: () => api.getAccessFeed(),
      enabled,
    });
  };
}

/**
 * Build `useConnectedApps` over one client's OAuth API: the outside apps the
 * athlete approved on the consent screen, under the one key every surface
 * that lists or revokes them reads.
 */
export function createConnectedAppsHook(oauthApi: Pick<OAuthApi, 'listConnectedApps'>) {
  return function useConnectedApps(enabled = true) {
    return useQuery({
      queryKey: QUERY_KEYS.oauth.connectedApps(),
      queryFn: () => oauthApi.listConnectedApps(),
      enabled,
    });
  };
}

/**
 * Build `useRevokeConnectedApps` over one client's OAuth API: revoke every
 * grant id it is given (a client may hold one grant per scope), then reread
 * the Connected-apps list and the access feed. It rereads them after a
 * failure too, since a revoke that failed part-way still revoked something.
 * Callers pass their own `onSuccess` / `onError` to `mutate`.
 */
export function createRevokeConnectedAppsHook(oauthApi: Pick<OAuthApi, 'revokeConnectedApp'>) {
  return function useRevokeConnectedApps() {
    const queryClient = useQueryClient();
    return useMutation({
      mutationFn: async (grantIds: readonly string[]) => {
        await Promise.all(grantIds.map((id) => oauthApi.revokeConnectedApp(id)));
      },
      onSettled: async () => {
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.oauth.connectedApps() }),
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.accessFeed.feed() }),
        ]);
      },
    });
  };
}

/** One outside app's reads in the feed, or the reads Dravr support made. */
export interface AccessFeedAppGroup {
  /** Stable per group, for a list key and a test id. */
  key: string;
  /** `support`: Dravr support's reads, whose client is the admin's and is never named. */
  kind: 'app' | 'support';
  /** The app's client id; null for support, whose client is the admin's, and when the read names none. */
  clientId: string | null;
  /** The app's name, its client id when it registered none; null when neither is known or for support. */
  name: string | null;
  host: string | null;
  credential: AccessFeedCredential | null;
  transport: AccessFeedTransport;
  /** Newest first, as the server sent them. */
  reads: AccessFeedAppRead[];
  /**
   * The athlete's active grants of this client, which Revoke revokes all of.
   * Empty when the app is not a Connected app, or no grant of it is left.
   */
  revokeGrantIds: string[];
}

/**
 * Group the feed's app reads per client, in the order of each one's newest
 * read. Revoke is offered only to a Connected app (`oauth_grant`) the athlete
 * still holds a grant of: an API key, an MCP token or an A2A client is managed
 * where it was made, and support's reads are the admin's.
 */
export function groupAccessFeedApps(
  reads: readonly AccessFeedAppRead[],
  grants: readonly OAuthGrant[],
): AccessFeedAppGroup[] {
  const groups = new Map<string, AccessFeedAppGroup>();
  for (const read of reads) {
    const key = read.support_read
      ? 'support'
      : `app:${read.client_id ?? ''}:${read.credential ?? ''}:${read.transport}`;
    const existing = groups.get(key);
    if (existing) {
      existing.reads.push(read);
      continue;
    }
    if (read.support_read) {
      groups.set(key, {
        key,
        kind: 'support',
        clientId: null,
        name: null,
        host: null,
        credential: null,
        transport: read.transport,
        reads: [read],
        revokeGrantIds: [],
      });
      continue;
    }
    const clientGrants = read.client_id ? grants.filter((grant) => grant.client_id === read.client_id) : [];
    groups.set(key, {
      key,
      kind: 'app',
      clientId: read.client_id,
      name: read.client_name ?? clientGrants[0]?.client_name ?? read.client_id,
      host: read.client_host ?? clientGrants[0]?.client_host ?? null,
      credential: read.credential,
      transport: read.transport,
      reads: [read],
      revokeGrantIds: read.credential === 'oauth_grant' ? clientGrants.map((grant) => grant.id) : [],
    });
  }
  return [...groups.values()];
}

/** Whether an app's access is cut off under API keys or MCP tokens, which the card points to. */
export function revokedInKeySettings(credential: AccessFeedCredential | null): boolean {
  return credential === 'api_key' || credential === 'mcp_token';
}

/** One of the athlete's API keys and the days its REST requests read their data. */
export interface AccessFeedApiKeyGroup {
  keyId: string;
  name: string;
  prefix: string;
  /** Newest first, as the server sent them. */
  days: AccessFeedApiKeyDay[];
}

/**
 * Group the feed's API-key days per key, in the order of each key's newest
 * day. A key is managed in the API-keys settings, so it carries no Revoke.
 */
export function groupAccessFeedApiKeyDays(days: readonly AccessFeedApiKeyDay[]): AccessFeedApiKeyGroup[] {
  const groups = new Map<string, AccessFeedApiKeyGroup>();
  for (const day of days) {
    const existing = groups.get(day.key_id);
    if (existing) {
      existing.days.push(day);
      continue;
    }
    groups.set(day.key_id, { keyId: day.key_id, name: day.key_name, prefix: day.key_prefix, days: [day] });
  }
  return [...groups.values()];
}
