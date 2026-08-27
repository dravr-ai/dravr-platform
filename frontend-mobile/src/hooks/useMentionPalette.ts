// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: @handle mention palette state for the mobile composer — installed coaches, matches, keyboard
// ABOUTME: Sibling of useCommandPalette: the composer renders what this decides and inserts what it drafts

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  QUERY_KEYS,
  insertMention,
  matchMentionCoaches,
  mentionDraftAt,
} from '@pierre/shared-constants';
import type { MentionCandidate } from '@pierre/shared-constants';
import { coachesApi } from '../services/api';
import { COMPOSER_KEYS, composerKey, type ComposerKeyEvent } from './composerKeys';

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
   * Handle a composer keystroke from a hardware keyboard. Returns true when
   * the palette consumed it, in which case the composer must not also act on it.
   */
  handleKeyPress: (event: ComposerKeyEvent) => boolean;
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

  // Only a coach the athlete has actually installed answers a mention:
  // `find_installed_by_handle` joins `coach_assignments` for this user, so a
  // catalogue coach nobody installed would be offered here and then silently
  // not route. `is_assigned` is that same join surfaced on the list row — and
  // it is the discriminator, not `is_system`: the resolver admits a system
  // coach (`OR c.is_system = 1`) once the athlete has been assigned it.
  const mentionable = useMemo(
    () => (data?.coaches ?? []).filter(coach => coach.is_assigned === true),
    [data],
  );

  const matches = useMemo(
    () => (dismissed || draft === null ? [] : matchMentionCoaches(mentionable, draft.query)),
    [mentionable, draft, dismissed],
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

  const handleKeyPress = useCallback(
    (event: ComposerKeyEvent): boolean => {
      if (matches.length === 0 || draft === null) return false;
      const key = composerKey(event);
      if (key === COMPOSER_KEYS.down) {
        setHighlightedIndex((i) => (i + 1) % matches.length);
        return true;
      }
      if (key === COMPOSER_KEYS.up) {
        setHighlightedIndex((i) => (i - 1 + matches.length) % matches.length);
        return true;
      }
      if (key === COMPOSER_KEYS.enter || key === COMPOSER_KEYS.tab) {
        const candidate = matches[highlightedIndex];
        // Enter on a handle already typed in full belongs to the composer:
        // the athlete addressed the coach and means to send the message.
        if (key === COMPOSER_KEYS.enter && draft.query === candidate.handle) {
          return false;
        }
        select(candidate);
        return true;
      }
      if (key === COMPOSER_KEYS.escape) {
        setDismissed(true);
        return true;
      }
      return false;
    },
    [matches, draft, highlightedIndex, select],
  );

  return { isOpen: matches.length > 0, matches, highlightedIndex, select, handleKeyPress };
}
