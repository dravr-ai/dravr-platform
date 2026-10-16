// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The query meta that keeps a session-only value out of the MMKV-persisted query cache
// ABOUTME: Read by QueryProvider's persistsQuery; carried by flags that must reset on the next launch

/**
 * The query `meta` a session-only value carries so the persister leaves it in
 * memory: a launch then starts from the query's own first answer.
 */
export const SESSION_ONLY_QUERY_META = { persist: false } as const;
