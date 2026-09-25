// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: @handle mention palette state for both composers — installed agents, matches, keys
// ABOUTME: Sibling of the command palette: the composer renders what this decides and inserts what it drafts

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  QUERY_KEYS,
  insertMention,
  matchMentionCoaches,
  mentionDraftAt,
} from '@pierre/shared-constants';
import type { MentionCandidate } from '@pierre/shared-constants';
import type { AgentsApi } from '@pierre/api-client';
import { PALETTE_KEYS } from './paletteKeys';

/** An agent joins or leaves the list through Discover, never mid-keystroke. */
const AGENT_LIST_STALE_MS = 5 * 60_000;

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
  /** True when there is at least one installed agent to offer for the draft. */
  isOpen: boolean;
  /** The matching installed agents, one per handle, in handle order. */
  matches: MentionCandidate[];
  /** Index into `matches` of the highlighted row. */
  highlightedIndex: number;
  /** Replace the draft with this agent's handle and close the palette. */
  select: (candidate: MentionCandidate) => void;
  /**
   * Handle a keystroke, named as the platform reports it. Returns true when
   * the palette consumed it, in which case the composer must not also act on it.
   */
  handleKey: (key: string) => boolean;
}

/**
 * Build `useMentionPalette` over one client's agents API: drive a `@handle`
 * palette from the composer's text and caret.
 *
 * The candidates are the athlete's own agent list — the same "installed" set
 * the server resolves a mention against (`find_installed_by_handle`), so the
 * palette never offers a handle the turn would ignore. A phone keyboard that
 * capitalises the letter after `@` still finds the agent, and the inserted
 * text is the handle as the catalogue spells it, followed by a space.
 *
 * The agent list is only fetched once the athlete has typed a `@`; a palette
 * nobody opens costs no request.
 */
export function createMentionPaletteHook(agentsApi: Pick<AgentsApi, 'list'>) {
  return function useMentionPalette({
    value,
    caret,
    onChange,
  }: UseMentionPaletteOptions): UseMentionPaletteResult {
    const [dismissed, setDismissed] = useState(false);
    const [highlightedIndex, setHighlightedIndex] = useState(0);
    const draft = useMemo(() => mentionDraftAt(value, caret), [value, caret]);

    const { data } = useQuery({
      queryKey: QUERY_KEYS.coaches.list(),
      queryFn: () => agentsApi.list(),
      enabled: draft !== null,
      staleTime: AGENT_LIST_STALE_MS,
    });

    // Only an agent the athlete has actually installed answers a mention:
    // `find_installed_by_handle` joins `coach_assignments` for this user, so a
    // catalogue agent nobody installed would be offered here and then silently
    // not route. `is_assigned` is that same join surfaced on the list row — and
    // it is the discriminator, not `is_system`: the resolver admits a system
    // agent (`OR c.is_system = 1`) once the athlete has been assigned it.
    const mentionable = useMemo(
      () => (data?.agents ?? []).filter(agent => agent.is_assigned === true),
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

    const handleKey = useCallback(
      (key: string): boolean => {
        if (matches.length === 0 || draft === null) return false;
        if (key === PALETTE_KEYS.down) {
          setHighlightedIndex((i) => (i + 1) % matches.length);
          return true;
        }
        if (key === PALETTE_KEYS.up) {
          setHighlightedIndex((i) => (i - 1 + matches.length) % matches.length);
          return true;
        }
        if (key === PALETTE_KEYS.enter || key === PALETTE_KEYS.tab) {
          const candidate = matches[highlightedIndex];
          // Enter on a handle already typed in full belongs to the composer:
          // the athlete addressed the agent and means to send the message.
          if (key === PALETTE_KEYS.enter && draft.query === candidate.handle) {
            return false;
          }
          select(candidate);
          return true;
        }
        if (key === PALETTE_KEYS.escape) {
          setDismissed(true);
          return true;
        }
        return false;
      },
      [matches, draft, highlightedIndex, select],
    );

    return { isOpen: matches.length > 0, matches, highlightedIndex, select, handleKey };
  };
}
