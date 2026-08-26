// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: @handle mention palette state for the mobile composer — the installed coaches and the matches
// ABOUTME: Sibling of useCommandPalette: the composer renders what this decides and inserts what it drafts

import { useCallback, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  QUERY_KEYS,
  insertMention,
  matchMentionCoaches,
  mentionDraftAt,
} from '@pierre/shared-constants';
import type { MentionCandidate } from '@pierre/shared-constants';
import { coachesApi } from '../services/api';

/** Inputs the composer hands the mention palette. */
export interface UseMentionPaletteOptions {
  /** Current composer text. */
  value: string;
  /** Caret offset into `value`; the palette opens on the token ending here. */
  caret: number;
}

/** The composer text and caret that inserting a handle produces. */
export interface MentionInsertion {
  value: string;
  caret: number;
}

/** What the composer needs to render and drive the mention palette. */
export interface UseMentionPaletteResult {
  /** The matching installed coaches, one per handle, in handle order. */
  matches: MentionCandidate[];
  /**
   * Replace the open draft with this coach's handle — lowercase, verbatim,
   * followed by a space — or null when no draft is open at the caret.
   */
  insert: (candidate: MentionCandidate) => MentionInsertion | null;
}

/**
 * Drive a `@handle` palette from the composer's text and caret.
 *
 * The candidates are the athlete's own coach list — the same "installed" set
 * the server resolves a mention against (`find_installed_by_handle`), so the
 * palette never offers a handle the turn would ignore. The grammar is the
 * shared one web uses: a phone keyboard that capitalises the letter after
 * `@` still finds the coach, and the inserted text is the handle as the
 * catalogue spells it.
 *
 * The coach list is only fetched once the athlete has typed a `@`; a palette
 * nobody opens costs no request.
 */
export function useMentionPalette({ value, caret }: UseMentionPaletteOptions): UseMentionPaletteResult {
  const draft = useMemo(() => mentionDraftAt(value, caret), [value, caret]);

  const { data } = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    enabled: draft !== null,
    // A coach joins or leaves the list through Discover, never mid-keystroke.
    staleTime: 5 * 60_000,
  });

  const matches = useMemo(
    () => (draft === null ? [] : matchMentionCoaches(data?.coaches ?? [], draft.query)),
    [data, draft],
  );

  const insert = useCallback(
    (candidate: MentionCandidate): MentionInsertion | null =>
      draft === null ? null : insertMention(value, draft, candidate.handle),
    [draft, value],
  );

  return { matches, insert };
}
