// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's team as Home draws it on both clients: who is on it, who leads, and the order of the others
// ABOUTME: Membership is `is_assigned` on the agent list, the lead the agent the personal thread is bound to

import type { Agent } from '@pierre/shared-types';

/** The athlete's team: its lead, when one is on it, and the other agents by title. */
export interface OrderedTeam {
  /** The agent the personal thread is bound to, or `null` when that agent is not on the team. */
  lead: Agent | null;
  /** Every other agent on the team, by title. */
  members: Agent[];
}

/**
 * The athlete's team, lead apart, then the others by title.
 *
 * The team is `is_assigned` on the agent list — the one membership rule the
 * mention palette and Discover read. The lead is the agent the personal
 * thread is bound to; a lead that is not on the team marks nobody.
 */
export function orderTeam(agents: readonly Agent[], leadId: string | null): OrderedTeam {
  const team = agents.filter((agent) => agent.is_assigned === true);
  const lead = leadId === null ? null : (team.find((agent) => agent.id === leadId) ?? null);
  const members = team.filter((agent) => agent !== lead).sort((a, b) => a.title.localeCompare(b.title));
  return { lead, members };
}

/** The whole team in the order a row of avatars and names reads it: lead first, then the others. */
export function teamInOrder({ lead, members }: OrderedTeam): Agent[] {
  return lead === null ? members : [lead, ...members];
}
