// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Slash-command palette state for the mobile composer — catalogue, matches, keyboard
// ABOUTME: Lives outside screens/chat so the composer only renders what this decides

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useLocalSearchParams } from 'expo-router';
import { QUERY_KEYS, commandDraftFor, matchCommands } from '@pierre/shared-constants';
import type { CommandEntry } from '@pierre/shared-types';
import { chatApi } from '../services/api';
import { NEW_CONVERSATION_ID } from '../navigation/routes';
import { COMPOSER_KEYS, composerKey, type ComposerKeyEvent } from './composerKeys';

/** Inputs the composer hands the palette. */
export interface UseCommandPaletteOptions {
  /** Current composer text. The palette opens and closes off this alone. */
  value: string;
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
   * Handle a composer keystroke from a hardware keyboard. Returns true when
   * the palette consumed it, in which case the composer must not also act on
   * it — Enter with the palette open completes a command, it does not send a
   * half-typed one.
   */
  handleKeyPress: (event: ComposerKeyEvent) => boolean;
}

/**
 * Drive a slash-command palette from the composer's text.
 *
 * The catalogue comes from the server, which resolves it per caller through
 * the same availability predicates `/help` asks each handler — so this never
 * offers a command the athlete would be refused. No command is named here: one
 * added to the server's catalogue appears with no client change.
 *
 * The conversation is read from the route rather than taken as an argument.
 * `conversationId` is the same param the chat screen selects its conversation
 * from, so the palette and the transcript always describe the same thread, and
 * the composer needs no new prop threaded through to get it. The sentinel
 * `'new'` means no conversation yet, which the server reads as "answer for the
 * caller's own memberships" — the right answer for a thread with no group.
 */
export function useCommandPalette({
  value,
  onChange,
}: UseCommandPaletteOptions): UseCommandPaletteResult {
  const params = useLocalSearchParams<{ conversationId?: string }>();
  const conversationId =
    params?.conversationId && params.conversationId !== NEW_CONVERSATION_ID
      ? params.conversationId
      : undefined;
  const [dismissed, setDismissed] = useState(false);
  const [highlightedIndex, setHighlightedIndex] = useState(0);
  const wantsCatalogue = value.trimStart().startsWith('/');

  const { data } = useQuery({
    queryKey: QUERY_KEYS.chat.commands(conversationId),
    queryFn: () => chatApi.listCommands(conversationId),
    enabled: wantsCatalogue,
    // The catalogue changes when the athlete's group standing changes, which
    // is rare and never mid-keystroke.
    staleTime: 5 * 60_000,
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

  const handleKeyPress = useCallback(
    (event: ComposerKeyEvent): boolean => {
      if (matches.length === 0) return false;
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
        const entry = matches[highlightedIndex];
        // Enter on a command that is already complete belongs to the composer:
        // the athlete typed the whole thing and means to send it.
        if (key === COMPOSER_KEYS.enter && value.trimStart().toLowerCase() === entry.command.toLowerCase()) {
          return false;
        }
        select(entry);
        return true;
      }
      if (key === COMPOSER_KEYS.escape) {
        setDismissed(true);
        return true;
      }
      return false;
    },
    [matches, highlightedIndex, select, value],
  );

  return { isOpen: matches.length > 0, matches, highlightedIndex, select, handleKeyPress };
}
