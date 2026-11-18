// ABOUTME: What the console tells of an admin token's standing: a rotated token beside a revoked one
// ABOUTME: A rotated token has a live replacement, so no revoke, single or bulk, is offered for it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { AdminToken } from '../types/api';

/**
 * A token a rotation retired: its replacement is live, so revoking it ends
 * nothing and the server refuses it (409, carnet#1589). A token rotated
 * before the server recorded the replacement reads as revoked.
 */
export function isRotated(token: Pick<AdminToken, 'is_active' | 'rotated_to_token_id'>): boolean {
  return !token.is_active && Boolean(token.rotated_to_token_id);
}
