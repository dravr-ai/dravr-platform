// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins when a composer's voice is not yet known — a routed thread not opened yet, or a room whose group is still read
// ABOUTME: Red if a coach's group room can again flash the athlete's "Message Dravr…" while it loads

import { describe, expect, it } from 'vitest';
import { coachComposerPlaceholder, composerVoicePending } from '../src/composer-voice';

const KNOWN = { routedId: 'conv-1', newThreadId: 'new', openedId: 'conv-1', groupId: null, groupLoading: false };

describe('composerVoicePending', () => {
  it('is pending while the routed thread is not the one opened', () => {
    expect(composerVoicePending({ ...KNOWN, openedId: null })).toBe(true);
    expect(composerVoicePending({ ...KNOWN, openedId: 'conv-previous' })).toBe(true);
  });

  it("is pending while a room's group is still read", () => {
    expect(composerVoicePending({ ...KNOWN, groupId: 'g1', groupLoading: true })).toBe(true);
  });

  it('is known once the thread and its group are read', () => {
    expect(composerVoicePending(KNOWN)).toBe(false);
    expect(composerVoicePending({ ...KNOWN, groupId: 'g1', groupLoading: false })).toBe(false);
  });

  it('is pending on a client that does not route by id while the group is read', () => {
    expect(composerVoicePending({ openedId: 'conv-1', groupId: 'g1', groupLoading: true })).toBe(true);
    expect(composerVoicePending({ openedId: 'conv-1', groupId: 'g1', groupLoading: false })).toBe(false);
  });

  it('is known for a thread not yet started', () => {
    expect(composerVoicePending({ ...KNOWN, routedId: undefined, openedId: null })).toBe(false);
    expect(composerVoicePending({ ...KNOWN, routedId: 'new', openedId: null })).toBe(false);
  });
});

describe('coachComposerPlaceholder', () => {
  const t = (key: string, params?: Record<string, string | number>) => `${key}${params ? JSON.stringify(params) : ''}`;

  it("names the athlete in a coach's plan thread, and the group in their room", () => {
    expect(coachComposerPlaceholder(t, { planSubjectName: 'Alice', coachRoomName: null, pending: false })).toBe(
      'humanCoach.planThread.composerPlaceholder{"name":"Alice"}',
    );
    expect(coachComposerPlaceholder(t, { planSubjectName: null, coachRoomName: 'Tri', pending: false })).toBe(
      'humanCoach.groupRoom.composerPlaceholder{"group":"Tri"}',
    );
  });

  it("prints nothing while the voice is not known, and leaves an athlete's thread its own words", () => {
    expect(coachComposerPlaceholder(t, { planSubjectName: null, coachRoomName: null, pending: true })).toBe('');
    expect(coachComposerPlaceholder(t, { planSubjectName: null, coachRoomName: null, pending: false })).toBeNull();
  });
});
