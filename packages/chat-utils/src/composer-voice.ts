// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Whose composer the thread on screen has — an athlete's or its coach's — its placeholder, and when it is too unread to say
// ABOUTME: Until it is known both clients show no placeholder, so a coach's room never flashes the athlete's "Message Dravr…"

import { GROUP_ROOM_KEYS, PLAN_THREAD_KEYS } from './plan-draft';
import type { Translate } from './text';

/** What a chat screen has read of the thread it shows. */
export interface ComposerVoiceInputs {
  /** The conversation the route names, when the client routes by id; absent for a thread not yet started. */
  routedId?: string;
  /** The id a client routes to for a thread not yet started. */
  newThreadId?: string;
  /** The conversation the screen has opened, once it has resolved the route. */
  openedId: string | null;
  /** The opened thread's group, when it is a group's room. */
  groupId: string | null;
  /** The group's read is still out. */
  groupLoading: boolean;
}

/**
 * Whether the composer's voice is not yet known. A coach's room and plan
 * thread speak to the coach, every other thread in the app's own words, and
 * which one this is depends on the conversation opened and, for a room, on
 * its group's coach. Until both are read the composer would print the
 * athlete's placeholder and then swap it, so it prints none.
 */
export function composerVoicePending({
  routedId,
  newThreadId,
  openedId,
  groupId,
  groupLoading,
}: ComposerVoiceInputs): boolean {
  const routedToThread = routedId !== undefined && routedId !== '' && routedId !== newThreadId;
  if (routedToThread && openedId !== routedId) return true;
  return groupId !== null && groupLoading;
}

/** Whose composer it is, as a chat screen has read it. */
export interface ComposerVoice {
  /** The athlete a coach's plan thread is about; `null` in every other thread. */
  planSubjectName: string | null;
  /** The group's name when the viewer is its human coach in its room; `null` for every other reader. */
  coachRoomName: string | null;
  /** The composer's voice is not yet known (`composerVoicePending`). */
  pending: boolean;
}

/**
 * The composer's placeholder in a coach's own thread: the plan thread names
 * the athlete the agent is asked about, the room names the group that reads
 * it. `''` while the voice is not yet known, so nothing is printed and then
 * swapped; `null` in an athlete's thread, which keeps the app's own words.
 */
export function coachComposerPlaceholder(t: Translate, voice: ComposerVoice): string | null {
  if (voice.planSubjectName !== null) return t(PLAN_THREAD_KEYS.composerPlaceholder, { name: voice.planSubjectName });
  if (voice.coachRoomName !== null) return t(GROUP_ROOM_KEYS.composerPlaceholder, { group: voice.coachRoomName });
  return voice.pending ? '' : null;
}
