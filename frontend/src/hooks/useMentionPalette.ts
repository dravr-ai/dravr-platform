// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: @handle mention palette state for the web composer — installed coaches, matches, keyboard
// ABOUTME: Sibling of useCommandPalette: the composer renders what this decides and nothing more

import { useCallback, useEffect, useMemo, useState } from 'react';
import type { KeyboardEvent } from 'react';
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
  /** Called with the text the composer should now hold and where its caret goes. */
  onChange: (value: string, caret: number) => void;
}

/** What the composer needs to render and drive the mention palette. */
export interface UseMentionPaletteResult {
  /** True when there is at least one installed coach to offer for the draft. */
  isOpen: boolean;
  /** The matching installed coaches, one per handle, in handle order. */
  matches: MentionCandidate[];
  /** Index into `matches` of the highlighted row. */
  highlightedIndex: number;
  /** Replace the draft with this coach's handle and close the palette. */
  select: (candidate: MentionCandidate) => void;
  /**
   * Handle a composer keystroke. Returns true when the palette consumed it,
   * in which case the composer must not also act on it.
   */
  handleKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => boolean;
}

/**
 * Drive a `@handle` palette from the composer's text and caret.
 *
 * The candidates are the athlete's own coach list — the same "installed"
 * set the server resolves a mention against, so the palette never offers a
 * handle the turn would ignore. The handle is inserted verbatim, in the
 * lowercase spelling the catalogue assigned, followed by a space.
 *
 * The coach list is only fetched once the athlete has typed a `@`; a palette
 * nobody opens costs no request.
 */
export function useMentionPalette({
  value,
  caret,
  onChange,
}: UseMentionPaletteOptions): UseMentionPaletteResult {
  const [dismissed, setDismissed] = useState(false);
  const [highlightedIndex, setHighlightedIndex] = useState(0);
  const draft = useMemo(() => mentionDraftAt(value, caret), [value, caret]);

  const { data } = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    enabled: draft !== null,
    // A coach joins or leaves the list through Discover, never mid-keystroke.
    staleTime: 5 * 60_000,
  });

  const matches = useMemo(
    () => (dismissed || draft === null ? [] : matchMentionCoaches(data?.coaches ?? [], draft.query)),
    [data, draft, dismissed],
  );

  // Escape dismisses the palette for the current draft only. The next
  // keystroke re-opens it, so the athlete is never locked out of a mention.
  useEffect(() => {
    setDismissed(false);
  }, [value]);

  useEffect(() => {
    setHighlightedIndex(0);
  }, [matches.length]);

  const select = useCallback(
    (candidate: MentionCandidate) => {
      if (draft === null) return;
      const next = insertMention(value, draft, candidate.handle);
      onChange(next.value, next.caret);
    },
    [draft, value, onChange],
  );

  const handleKeyDown = useCallback(
    (event: KeyboardEvent<HTMLTextAreaElement>): boolean => {
      if (matches.length === 0 || draft === null) return false;
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        setHighlightedIndex((i) => (i + 1) % matches.length);
        return true;
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault();
        setHighlightedIndex((i) => (i - 1 + matches.length) % matches.length);
        return true;
      }
      if (event.key === 'Enter' || event.key === 'Tab') {
        const candidate = matches[highlightedIndex];
        // Enter on a handle already typed in full belongs to the composer:
        // the athlete addressed the coach and means to send the message.
        if (event.key === 'Enter' && draft.query === candidate.handle) {
          return false;
        }
        event.preventDefault();
        select(candidate);
        return true;
      }
      if (event.key === 'Escape') {
        event.preventDefault();
        setDismissed(true);
        return true;
      }
      return false;
    },
    [matches, draft, highlightedIndex, select],
  );

  return {
    isOpen: matches.length > 0,
    matches,
    highlightedIndex,
    select,
    handleKeyDown,
  };
}
