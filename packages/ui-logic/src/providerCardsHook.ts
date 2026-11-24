// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query read of the provider cards, per client's OAuth API, on the one provider-status key
// ABOUTME: A screen that connects, disconnects or reconnects reads its rows here and reads them again through `reload`

import { useCallback } from 'react';
import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { OAuthApi } from '@pierre/api-client';
import type { ExtendedProviderStatus } from '@pierre/shared-types';

/** The list a screen shows before any read answered; one identity, so it never re-runs an effect. */
const NO_CARDS: ExtendedProviderStatus[] = [];

/** How one screen reads the cards. */
export interface ProviderCardsOptions {
  /** False holds the read back, for a screen that can render signed out. */
  enabled?: boolean;
  /**
   * `'always'` reads again every time the screen opens, whatever the cache
   * holds: a screen whose whole subject is the connections never opens on a
   * list another surface left behind.
   */
  refetchOnMount?: boolean | 'always';
}

/** What {@link createProviderCardsHook}'s hook hands a screen. */
export interface ProviderCards {
  /** The cards as the server last listed them; empty until a read answers. */
  providers: ExtendedProviderStatus[];
  /** True once a read has answered, so an empty list is not taken for "nothing connected". */
  loaded: boolean;
  /** True while no read has answered yet: the screen's loading state. */
  isPending: boolean;
  /** Why the cards could not be read, while there is no list to show; `null` otherwise. */
  loadError: Error | null;
  /**
   * Read the cards again after a change made here (a connect that came back,
   * a disconnect), replacing a re-read in flight, which may predate the
   * change. Resolves to the cards read, none when the read failed, so the
   * caller can judge the row the change landed on.
   */
  reload: () => Promise<ExtendedProviderStatus[]>;
  /** Bring the cards up to date when the screen comes back; joins a read already in flight. */
  revalidate: () => Promise<void>;
}

/**
 * Build `useProviderCards` over one client's OAuth API: the provider cards,
 * read on the provider-status key every other reader of them shares. A
 * change made anywhere — a consent given, a link confirmed in a group —
 * invalidates that key, and every screen holding this hook shows the
 * server's answer without asking for it.
 */
export function createProviderCardsHook(oauthApi: Pick<OAuthApi, 'getProvidersStatus'>) {
  return function useProviderCards(options: ProviderCardsOptions = {}): ProviderCards {
    const { data, error, isPending, refetch } = useQuery({
      queryKey: QUERY_KEYS.providers.status(),
      queryFn: () => oauthApi.getProvidersStatus(),
      enabled: options.enabled ?? true,
      refetchOnMount: options.refetchOnMount ?? true,
    });
    const reload = useCallback(async () => {
      const result = await refetch();
      return result.isError ? NO_CARDS : (result.data?.providers ?? NO_CARDS);
    }, [refetch]);
    const revalidate = useCallback(async () => {
      await refetch({ cancelRefetch: false });
    }, [refetch]);
    return {
      providers: data?.providers ?? NO_CARDS,
      loaded: data !== undefined,
      isPending,
      loadError: data === undefined ? error : null,
      reload,
      revalidate,
    };
  };
}
