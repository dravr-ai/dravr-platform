// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's binding of the consent-history and standing-policy hooks — every receipt a page at a time, the policies and their removal
// ABOUTME: Connected apps lists the apps the athlete always allows and the consent history under its apps (carnet#1544)

import { createGrantReceiptsHook, createRemoveStandingPolicyHook, createStandingPoliciesHook } from '@pierre/ui-logic';
import { oauthApi } from '../services/api';

/** One page of every receipt of the athlete's grants, newest first. */
export const useGrantReceipts = createGrantReceiptsHook({
  listGrantReceipts: (page) => oauthApi.listGrantReceipts(page),
});

/** The apps the athlete always allows. */
export const useStandingPolicies = createStandingPoliciesHook({
  listStandingPolicies: () => oauthApi.listStandingPolicies(),
});

/** Stop always allowing an app, then reread the policies. */
export const useRemoveStandingPolicy = createRemoveStandingPolicyHook({
  removeStandingPolicy: (id) => oauthApi.removeStandingPolicy(id),
});
