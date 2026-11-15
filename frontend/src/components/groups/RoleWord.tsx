// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The word under a Group info row's name saying who that person is — owner, admin, member, the AI agent, the coach
// ABOUTME: One table for all five, so the agent and the human coach read as part of the same list as the members

import type { GroupRole } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';

/**
 * Who a row is: a member by their group role, or one of the two who run the
 * group without a membership row — the AI agent and the human coach.
 */
export type RosterKind = GroupRole | 'agent' | 'coach';

// Built at import time, where `t` does not exist: the table carries the key
// and the render resolves it.
const ROSTER_LABEL_KEY: Record<RosterKind, string> = {
  owner: 'groups.owner',
  admin: 'groups.admin',
  member: 'groups.member',
  agent: 'app.aiAgent',
  coach: 'humanCoach.coach',
};

/**
 * The role as a plain word in the row's second line, never a tinted pill:
 * the word is the information, and a hue per role would put five colours in
 * a list of four people.
 */
export default function RoleWord({ kind }: { kind: RosterKind }) {
  const { t } = useTranslation();
  return <span>{t(ROSTER_LABEL_KEY[kind])}</span>;
}
