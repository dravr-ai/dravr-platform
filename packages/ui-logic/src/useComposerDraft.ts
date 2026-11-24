// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Both clients' unsent composer line, kept per thread — opening another chat shows that chat's own draft
// ABOUTME: One store outlives the screen that typed it, scoped to the signed-in user; "new chat" is a chat too, its line moving only into a thread made for it

import { useCallback, useSyncExternalStore } from 'react';

/** One thread's unsent line, and whether a draft affordance put it there. */
export interface ComposerDraft {
  text: string;
  /**
   * The line came from a draft affordance (Home's Today, "back up this claim",
   * a route's draft), so its send is reported as a draft rather than as typed.
   */
  seeded: boolean;
}

/** What a composer reads and writes for the thread on screen. */
export interface ComposerDraftHandle extends ComposerDraft {
  /** The athlete typed: the line changes, its origin stays until it is emptied. */
  setText: (next: string | ((current: string) => string)) => void;
  /** Put a whole line in the composer, saying whether it is a draft affordance's. */
  seed: (text: string, seeded: boolean) => void;
}

const EMPTY_DRAFT: ComposerDraft = { text: '', seeded: false };

/** The slot a composer writes to while no thread is open yet. */
const NO_THREAD = '';

const drafts = new Map<string, ComposerDraft>();
const listeners = new Set<() => void>();

function slotOf(ownerId: string | null, threadId: string | null): string {
  // A thread id never holds the separator, so no owner/thread pair collides.
  return `${ownerId ?? ''}\u0000${threadId ?? NO_THREAD}`;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function store(slot: string, draft: ComposerDraft): void {
  const held = drafts.get(slot) ?? EMPTY_DRAFT;
  // An emptied line is no longer anyone's draft: what is typed next is the athlete's own.
  const next = draft.text === '' ? EMPTY_DRAFT : draft;
  if (held.text === next.text && held.seeded === next.seeded) return;
  if (next === EMPTY_DRAFT) drafts.delete(slot);
  else drafts.set(slot, next);
  listeners.forEach(listener => listener());
}

function readDraft(slot: string): ComposerDraft {
  return drafts.get(slot) ?? EMPTY_DRAFT;
}

/**
 * Write one thread's line from outside its composer, whichever thread is on
 * screen: a turn the server refused hands the typed words back to the thread
 * they were typed in, and a navigation that carries a draft fills the thread
 * it opens before that thread is the one drawn.
 */
export function writeComposerDraft(
  ownerId: string | null,
  threadId: string | null,
  next: (current: ComposerDraft) => ComposerDraft,
): void {
  const slot = slotOf(ownerId, threadId);
  store(slot, next(readDraft(slot)));
}

/**
 * Move the new-chat line into the thread a composer just opened for it — a
 * line drafted with no thread open, then a thread created to carry it. The
 * new-chat composer is empty again afterwards. A thread that already holds a
 * line of its own keeps it, and the new-chat line stays where it was typed.
 */
export function carryNoThreadDraft(ownerId: string | null, threadId: string): void {
  const fromSlot = slotOf(ownerId, null);
  const carried = readDraft(fromSlot);
  const slot = slotOf(ownerId, threadId);
  if (carried.text === '' || readDraft(slot).text !== '') return;
  store(slot, carried);
  store(fromSlot, EMPTY_DRAFT);
}

/**
 * The composer line of the thread on screen.
 *
 * Every thread keeps its own unsent line: opening another chat shows that
 * chat's draft, and coming back shows this one again. The lines live in one
 * store rather than in the screen, so a host that unmounts — leaving the Groups
 * tab for Home — finds them again, and two hosts on the same thread share one.
 *
 * The composer with no thread open is a chat of its own, "new chat": opening
 * an existing thread leaves its line there. Only a thread created for that
 * line takes it, through {@link carryNoThreadDraft}.
 */
export function useComposerDraft(ownerId: string | null, threadId: string | null): ComposerDraftHandle {
  const slot = slotOf(ownerId, threadId);
  const draft = useSyncExternalStore(
    subscribe,
    () => readDraft(slot),
    () => readDraft(slot),
  );

  const setText = useCallback(
    (next: string | ((current: string) => string)) => {
      const held = readDraft(slot);
      store(slot, { text: typeof next === 'function' ? next(held.text) : next, seeded: held.seeded });
    },
    [slot],
  );
  const seed = useCallback((text: string, seeded: boolean) => store(slot, { text, seeded }), [slot]);

  return { text: draft.text, seeded: draft.seeded, setText, seed };
}
