// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks for a Connected app's grant history and the athlete's standing policies, per client's API
// ABOUTME: Each grant's signed receipts, every receipt a page at a time, the policies, and a removal that rereads them (carnet#1544)

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { OAuthApi } from '@pierre/api-client';

/** Receipts a page of the grant history holds. */
export const GRANT_RECEIPT_PAGE = 50;

/**
 * Build `useConnectedAppReceipts` over one client's OAuth API: one grant's
 * signed receipts, newest first. Disabled until a grant is named.
 */
export function createConnectedAppReceiptsHook(oauthApi: Pick<OAuthApi, 'listConnectedAppReceipts'>) {
  return function useConnectedAppReceipts(grantId: string | null) {
    return useQuery({
      queryKey: QUERY_KEYS.oauth.connectedAppReceipts(grantId ?? ''),
      queryFn: () => oauthApi.listConnectedAppReceipts(grantId ?? ''),
      enabled: grantId !== null,
    });
  };
}

/**
 * Build `useGrantReceipts` over one client's OAuth API: page `page` (from 0)
 * of every receipt of the athlete's grants, newest first, including those of
 * grants since revoked or deleted with their client.
 */
export function createGrantReceiptsHook(oauthApi: Pick<OAuthApi, 'listGrantReceipts'>) {
  return function useGrantReceipts(page = 0, enabled = true) {
    const offset = page * GRANT_RECEIPT_PAGE;
    return useQuery({
      queryKey: QUERY_KEYS.oauth.grantReceipts(GRANT_RECEIPT_PAGE, offset),
      queryFn: () => oauthApi.listGrantReceipts({ limit: GRANT_RECEIPT_PAGE, offset }),
      enabled,
    });
  };
}

/** Build `useStandingPolicies` over one client's OAuth API: the athlete's live standing policies. */
export function createStandingPoliciesHook(oauthApi: Pick<OAuthApi, 'listStandingPolicies'>) {
  return function useStandingPolicies(enabled = true) {
    return useQuery({
      queryKey: QUERY_KEYS.oauth.standingPolicies(),
      queryFn: () => oauthApi.listStandingPolicies(),
      enabled,
    });
  };
}

/**
 * Build `useRemoveStandingPolicy` over one client's OAuth API: remove one
 * policy, then reread the policies, after a failure too. The grants it
 * renewed stay, so Connected apps is not reread.
 */
export function createRemoveStandingPolicyHook(oauthApi: Pick<OAuthApi, 'removeStandingPolicy'>) {
  return function useRemoveStandingPolicy() {
    const queryClient = useQueryClient();
    return useMutation({
      mutationFn: (policyId: string) => oauthApi.removeStandingPolicy(policyId),
      onSettled: async () => {
        await queryClient.invalidateQueries({ queryKey: QUERY_KEYS.oauth.standingPolicies() });
      },
    });
  };
}
