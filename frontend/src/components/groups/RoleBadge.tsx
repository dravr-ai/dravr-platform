// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The role pill every group roster row carries — owner, admin, member, the AI agent, the human coach
// ABOUTME: One shape and one table for all five, so the agent and the coach read as part of the same roster

import { clsx } from 'clsx';
import { Bot, Crown, Shield, User, UserCog, type LucideIcon } from 'lucide-react';
import type { GroupRole } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';

/**
 * Who a roster row is: a member by their group role, or one of the two who
 * run the group without a membership row — the AI agent and the human coach.
 */
export type RosterKind = GroupRole | 'agent' | 'coach';

// Built at import time, where `t` does not exist: the table carries the key
// and the render resolves it. Each tone is a token pair — a tint and the ink
// that reads on it — so the pill holds contrast in both themes.
const ROSTER_BADGE: Record<RosterKind, { labelKey: string; tone: string; Icon: LucideIcon }> = {
  owner: { labelKey: 'groups.owner', tone: 'bg-warning/20 text-on-warning-container', Icon: Crown },
  admin: { labelKey: 'groups.admin', tone: 'bg-primary/20 text-primary', Icon: Shield },
  member: { labelKey: 'groups.member', tone: 'bg-surface-container-high/20 text-on-surface-variant', Icon: User },
  agent: { labelKey: 'app.aiAgent', tone: 'bg-info/15 text-on-info-container', Icon: Bot },
  coach: { labelKey: 'humanCoach.coach', tone: 'bg-primary-container text-on-primary-container', Icon: UserCog },
};

/** The pill naming what a roster row's person is in the group. */
export default function RoleBadge({ kind }: { kind: RosterKind }) {
  const { t } = useTranslation();
  const { labelKey, tone, Icon } = ROSTER_BADGE[kind];
  return (
    <span
      className={clsx(
        'inline-flex shrink-0 items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium',
        tone,
      )}
    >
      <Icon className="w-3 h-3" aria-hidden="true" />
      {t(labelKey)}
    </span>
  );
}
