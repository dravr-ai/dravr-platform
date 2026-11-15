// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach-access request types — a coach asks for coach access, a super-admin grants or declines
// ABOUTME: Wire shape of GET/POST /api/me/coach-access-request and the admin console queue (carnet#738)

/** Where a coach-access request stands. */
export type CoachAccessStatus = 'pending' | 'granted' | 'declined';

/**
 * A coach's request for coach access (`manages_roster`). The request grants
 * nothing by itself (ADR-018): a super-admin decides. A grant also attaches
 * the coach to the group named here when it still has no coach.
 */
export interface CoachAccessRequest {
  id: string;
  user_id: string;
  /** The group a grant attaches the coach to, if they asked from one. */
  group_id: string | null;
  group_tenant_id: string | null;
  status: CoachAccessStatus;
  created_at: string;
  decided_at: string | null;
  decided_by: string | null;
  /**
   * The reason the super-admin gave with a decline, in their own words;
   * `null` for a decline given without one and for every other status.
   * Absent from a server that predates it.
   */
  decline_reason?: string | null;
  /**
   * When a newer ask from the same coach replaced this decline; the caller's
   * own read never returns a superseded decline, the console's history does.
   */
  superseded_at?: string | null;
}

/**
 * Body of the caller-side coach-access routes. After a withdrawal it is the
 * caller's latest request left — an earlier decided one, or `null`.
 */
export interface CoachAccessRequestResponse {
  /** The caller's latest request; `null` when they never asked. */
  request: CoachAccessRequest | null;
}

/** A request as the admin console queue shows it: who asked, for which group. */
export interface CoachAccessRequestView extends CoachAccessRequest {
  email: string | null;
  display_name: string | null;
  group_name: string | null;
}
