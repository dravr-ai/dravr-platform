// ABOUTME: The chat header's subtitle rule — group before handle, decided once for both clients
// ABOUTME: The two headers disagreed on precedence, so one thread described itself two ways

import { describe, expect, it } from 'vitest';
import { threadSubtitle } from '../src/thread-subtitle';

describe('threadSubtitle', () => {
  /**
   * The case the two clients answered differently: web read the group badge
   * first, mobile the coach handle. A group thread with a coach attached is
   * still a group thread — that is the fact about the room.
   */
  it('names the group when a thread has both a group and a coach', () => {
    expect(
      threadSubtitle({ group_name: 'Sunday Long Run', coach_handle: 'trail' }),
    ).toEqual({ kind: 'group' });
  });

  it('names the coach handle for a one-to-one thread', () => {
    expect(threadSubtitle({ group_name: null, coach_handle: 'trail' })).toEqual({
      kind: 'handle',
      handle: 'trail',
    });
  });

  it('says nothing for a plain thread, so the client can fall back', () => {
    // Web shows connected-provider status in that space; returning a kind here
    // would take the decision away from it.
    expect(threadSubtitle({ group_name: null, coach_handle: null })).toBeNull();
  });

  it('says nothing when there is no conversation at all', () => {
    expect(threadSubtitle(null)).toBeNull();
    expect(threadSubtitle(undefined)).toBeNull();
  });
});
