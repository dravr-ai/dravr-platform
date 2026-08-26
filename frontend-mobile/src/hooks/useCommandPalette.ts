// ABOUTME: Slash-command palette state for the mobile composer — catalogue and matches
// ABOUTME: Lives outside screens/chat so the composer only renders what this decides

import { useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';
import { useLocalSearchParams } from 'expo-router';
import {
  QUERY_KEYS,
  commandDraftFor,
  matchCommands,
} from '../../../packages/shared-constants/src/index';
import type { CommandEntry } from '@pierre/shared-types';
import { chatApi } from '../services/api';

/** What the composer needs to render and drive the palette. */
export interface UseCommandPaletteResult {
  /** The matching commands, in the server's domain-then-command order. */
  matches: CommandEntry[];
  /** The composer text that selecting `entry` produces. */
  draftFor: (entry: CommandEntry) => string;
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
export function useCommandPalette(value: string): UseCommandPaletteResult {
  const params = useLocalSearchParams<{ conversationId?: string }>();
  const conversationId =
    params?.conversationId && params.conversationId !== 'new' ? params.conversationId : undefined;
  const wantsCatalogue = value.trimStart().startsWith('/');

  const { data } = useQuery({
    queryKey: QUERY_KEYS.chat.commands(conversationId),
    queryFn: () => chatApi.listCommands(conversationId),
    enabled: wantsCatalogue,
    // The catalogue changes when the athlete's group standing changes, which
    // is rare and never mid-keystroke.
    staleTime: 5 * 60_000,
  });

  const matches = useMemo(() => matchCommands(data ?? [], value), [data, value]);

  return { matches, draftFor: commandDraftFor };
}
