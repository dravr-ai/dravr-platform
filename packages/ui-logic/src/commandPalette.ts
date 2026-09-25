// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Slash-command palette state for both composers — the server's catalogue, the matches, the keys
// ABOUTME: Platform-free: each composer reads its own key event and hands this the key's name

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS, commandDraftFor, matchCommands } from '@pierre/shared-constants';
import type { ChatApi } from '@pierre/api-client';
import type { CommandEntry } from '@pierre/shared-types';
import { PALETTE_KEYS } from './paletteKeys';

/** The catalogue changes when the athlete's group standing does — rarely, and never mid-keystroke. */
const CATALOGUE_STALE_MS = 5 * 60_000;

/** Inputs the composer hands the palette. */
export interface UseCommandPaletteOptions {
  /** Current composer text. The palette opens and closes off this alone. */
  value: string;
  /** The open conversation, so group-scoped commands answer for its group. */
  conversationId?: string | null;
  /** Called with the text the composer should now hold. */
  onChange: (value: string) => void;
}

/** What the composer needs to render and drive the palette. */
export interface UseCommandPaletteResult {
  /** True when there is at least one command to offer for the current text. */
  isOpen: boolean;
  /** The matching commands, in the server's domain-then-command order. */
  matches: CommandEntry[];
  /** Index into `matches` of the highlighted row. */
  highlightedIndex: number;
  /** Fill the composer with this command and keep its signature on screen. */
  select: (entry: CommandEntry) => void;
  /**
   * Handle a keystroke, named as the platform reports it (`ArrowDown`,
   * `Enter`, …). Returns true when the palette consumed it, in which case the
   * composer must not also act on it — Enter with the palette open completes a
   * command, it does not send a half-typed one.
   */
  handleKey: (key: string) => boolean;
}

/**
 * Build `useCommandPalette` over one client's chat API.
 *
 * The catalogue is fetched from the server, which resolves it per caller
 * through the same predicates `/help` asks — so this never offers a command
 * the athlete would be refused. Nothing about the list is hardcoded here: a
 * command added to the server's catalogue appears with no client change.
 *
 * The query only runs once the athlete has typed a `/`. A palette nobody opens
 * costs no request.
 */
export function createCommandPaletteHook(chatApi: Pick<ChatApi, 'listCommands'>) {
  return function useCommandPalette({
    value,
    conversationId,
    onChange,
  }: UseCommandPaletteOptions): UseCommandPaletteResult {
    const [dismissed, setDismissed] = useState(false);
    const [highlightedIndex, setHighlightedIndex] = useState(0);
    const wantsCatalogue = value.trimStart().startsWith('/');

    const { data } = useQuery({
      queryKey: QUERY_KEYS.chat.commands(conversationId),
      queryFn: () => chatApi.listCommands(conversationId ?? undefined),
      enabled: wantsCatalogue,
      staleTime: CATALOGUE_STALE_MS,
    });

    const matches = useMemo(
      () => (dismissed ? [] : matchCommands(data ?? [], value)),
      [data, value, dismissed],
    );

    // Escape dismisses the palette for the current draft only. Typing anything
    // else re-opens it, so the athlete is never locked out of their own commands.
    useEffect(() => {
      setDismissed(false);
    }, [value]);

    useEffect(() => {
      setHighlightedIndex(0);
    }, [matches.length]);

    // Selecting fills the composer and leaves the palette showing that one
    // command, so its argument signature stays visible while the athlete types
    // the arguments. Enter then belongs to the composer, not the palette.
    const select = useCallback(
      (entry: CommandEntry) => {
        onChange(commandDraftFor(entry));
      },
      [onChange],
    );

    const handleKey = useCallback(
      (key: string): boolean => {
        if (matches.length === 0) return false;
        if (key === PALETTE_KEYS.down) {
          setHighlightedIndex((i) => (i + 1) % matches.length);
          return true;
        }
        if (key === PALETTE_KEYS.up) {
          setHighlightedIndex((i) => (i - 1 + matches.length) % matches.length);
          return true;
        }
        if (key === PALETTE_KEYS.enter || key === PALETTE_KEYS.tab) {
          const entry = matches[highlightedIndex];
          // Enter on a command that is already complete belongs to the composer:
          // the athlete typed the whole thing and means to send it.
          if (
            key === PALETTE_KEYS.enter &&
            value.trimStart().toLowerCase() === entry.command.toLowerCase()
          ) {
            return false;
          }
          select(entry);
          return true;
        }
        if (key === PALETTE_KEYS.escape) {
          setDismissed(true);
          return true;
        }
        return false;
      },
      [matches, highlightedIndex, select, value],
    );

    return { isOpen: matches.length > 0, matches, highlightedIndex, select, handleKey };
  };
}
