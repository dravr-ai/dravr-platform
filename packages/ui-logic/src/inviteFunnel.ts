// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach Home's invite funnel on both clients — link sent, opened, account created, then not yet in the group or the group full
// ABOUTME: One pure reading of the server's funnel counts, so a coach can tell "never opened" from "signed up, never joined" (carnet#1650)

import type { GroupInviteFunnel } from '@pierre/shared-types';

/** The words of the funnel, every one under `humanCoach.inviteFunnel.*`. */
export const INVITE_FUNNEL_KEYS = {
  label: 'humanCoach.inviteFunnel.label',
  sent: 'humanCoach.inviteFunnel.sent',
  notOpened: 'humanCoach.inviteFunnel.notOpened',
  opened: 'humanCoach.inviteFunnel.opened',
  accountCreated: 'humanCoach.inviteFunnel.accountCreated',
  notJoined: 'humanCoach.inviteFunnel.notJoined',
  groupFull: 'humanCoach.inviteFunnel.groupFull',
} as const;

/** Which step of the funnel a row stands for. */
export type InviteFunnelStepKey = 'sent' | 'opened' | 'account' | 'joined';

/**
 * Where a step stands: `done` is behind the invite, `current` is the one it
 * waits on, `todo` is ahead of it.
 */
export type InviteFunnelStepState = 'done' | 'current' | 'todo';

/** One step of the funnel, ready to render: an i18n key and its values. */
export interface InviteFunnelStep {
  key: InviteFunnelStepKey;
  state: InviteFunnelStepState;
  /** A key of `INVITE_FUNNEL_KEYS` */
  labelKey: (typeof INVITE_FUNNEL_KEYS)[keyof typeof INVITE_FUNNEL_KEYS];
  /**
   * The label's values. `count` drives the plural (`opened`,
   * `accountCreated`), and `date` is the ISO time the caller formats in the
   * reader's language before passing it to `t`.
   */
  values: { count?: number; date?: string };
  /** The step's tone: `warn` for a full group, which the coach can act on. */
  tone: 'default' | 'warn';
}

/**
 * The four steps a pending invite's row shows: sent, opened, account created,
 * joined. Sent is always done (the row exists because the link was made).
 * Opened is done once the card was opened. Account created is done once an
 * account holds the invite, and the last step then waits on the join, or on a
 * seat when every holding account was refused because the group is full.
 * Without a funnel (only the list read carries one, never the invite a create
 * returns), the row reads as sent and nothing more is claimed.
 */
export function inviteFunnelSteps(funnel: GroupInviteFunnel | undefined): InviteFunnelStep[] {
  const opened = (funnel?.open_count ?? 0) > 0;
  const held = (funnel?.held_count ?? 0) > 0;
  const allFull = held && funnel !== undefined && funnel.full_count >= funnel.held_count;

  const sent: InviteFunnelStep = {
    key: 'sent',
    state: 'done',
    labelKey: INVITE_FUNNEL_KEYS.sent,
    values: {},
    tone: 'default',
  };
  // An account can hold the invite without the card's count (a code typed on
  // the waiting screen): it was opened, one way or another.
  const openedStep: InviteFunnelStep =
    opened || held
      ? {
          key: 'opened',
          state: 'done',
          labelKey: INVITE_FUNNEL_KEYS.opened,
          values: { count: Math.max(funnel?.open_count ?? 0, 1) },
          tone: 'default',
        }
      : {
          key: 'opened',
          state: 'current',
          labelKey: INVITE_FUNNEL_KEYS.notOpened,
          values: {},
          tone: 'default',
        };
  const account: InviteFunnelStep = {
    key: 'account',
    state: held ? 'done' : opened ? 'current' : 'todo',
    labelKey: INVITE_FUNNEL_KEYS.accountCreated,
    values: held
      ? { count: funnel?.held_count ?? 1, ...(funnel?.last_held_at ? { date: funnel.last_held_at } : {}) }
      : { count: 1 },
    tone: 'default',
  };
  const joined: InviteFunnelStep = allFull
    ? { key: 'joined', state: 'current', labelKey: INVITE_FUNNEL_KEYS.groupFull, values: {}, tone: 'warn' }
    : {
        key: 'joined',
        state: held ? 'current' : 'todo',
        labelKey: INVITE_FUNNEL_KEYS.notJoined,
        values: {},
        tone: 'default',
      };
  return [sent, openedStep, account, joined];
}
