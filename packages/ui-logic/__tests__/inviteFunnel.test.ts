// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the invite funnel a coach reads: never opened, opened, signed up and not joined, and a full group
// ABOUTME: Each state is told apart, and no step is claimed that the server's counts do not show (carnet#1650)

import { describe, expect, it } from 'vitest';
import type { GroupInviteFunnel } from '@pierre/shared-types';
import { INVITE_FUNNEL_KEYS, inviteFunnelSteps } from '../src/inviteFunnel';

function funnel(overrides: Partial<GroupInviteFunnel> = {}): GroupInviteFunnel {
  return {
    open_count: 0,
    last_opened_at: null,
    held_count: 0,
    last_held_at: null,
    full_count: 0,
    ...overrides,
  };
}

const states = (f: GroupInviteFunnel | undefined) => inviteFunnelSteps(f).map((step) => `${step.key}:${step.state}`);

describe('inviteFunnelSteps', () => {
  it('reads a link nobody opened as sent, waiting on its first opening', () => {
    expect(states(funnel())).toEqual(['sent:done', 'opened:current', 'account:todo', 'joined:todo']);
    expect(inviteFunnelSteps(funnel())[1]?.labelKey).toBe(INVITE_FUNNEL_KEYS.notOpened);
  });

  it('counts the openings and waits on an account once the card was opened', () => {
    const steps = inviteFunnelSteps(funnel({ open_count: 2, last_opened_at: '2026-11-14T23:59:21Z' }));
    expect(steps.map((s) => s.state)).toEqual(['done', 'done', 'current', 'todo']);
    expect(steps[1]).toMatchObject({ labelKey: INVITE_FUNNEL_KEYS.opened, values: { count: 2 } });
  });

  it('tells an athlete who signed up and never joined from one who never opened the link', () => {
    const steps = inviteFunnelSteps(funnel({ open_count: 2, held_count: 1, last_held_at: '2026-11-15T00:00:07Z' }));
    expect(steps.map((s) => s.state)).toEqual(['done', 'done', 'done', 'current']);
    expect(steps[2]).toMatchObject({ values: { count: 1, date: '2026-11-15T00:00:07Z' } });
    expect(steps[3]).toMatchObject({ labelKey: INVITE_FUNNEL_KEYS.notJoined, tone: 'default' });
  });

  it('shows the full group, not "account created", when every holding account was refused for a seat', () => {
    const steps = inviteFunnelSteps(funnel({ open_count: 1, held_count: 1, full_count: 1 }));
    expect(steps[3]).toMatchObject({ labelKey: INVITE_FUNNEL_KEYS.groupFull, state: 'current', tone: 'warn' });
  });

  it('keeps waiting on the join while one holding account was not refused', () => {
    const steps = inviteFunnelSteps(funnel({ open_count: 3, held_count: 2, full_count: 1 }));
    expect(steps[3]?.labelKey).toBe(INVITE_FUNNEL_KEYS.notJoined);
  });

  it('reads a code typed on the waiting screen, with no card opening, as opened', () => {
    const steps = inviteFunnelSteps(funnel({ held_count: 1 }));
    expect(steps[1]).toMatchObject({ state: 'done', values: { count: 1 } });
  });

  it('claims nothing past "sent" without a funnel', () => {
    expect(states(undefined)).toEqual(['sent:done', 'opened:current', 'account:todo', 'joined:todo']);
  });
});
