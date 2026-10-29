// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks behind a coach's private lane in a one-athlete group room — the switch, the private rows, the quote, the send
// ABOUTME: Both clients bind it once; whether the switch shows and where a private turn goes is decided here (carnet#1249)

import { useCallback, useMemo, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import {
  replySceneBlocks,
  TurnFailedError,
  TurnRequestError,
  type ChatApi,
  type MessagesResponse,
  type TurnCallbacks,
} from '@pierre/api-client';
import {
  filterDisplayMessages,
  PLAN_DRAFT_KEYS,
  PRIVATE_LANE_KEYS,
  roomQuoteOf,
  type QuotableRow,
  type Translate,
} from '@pierre/chat-utils';
import {
  isRoomQuoteRefusalReason,
  type CoachingGroup,
  type Conversation,
  type Message,
  type MessageAudience,
  type RoomQuote,
  type SoloAthlete,
  type TurnEnvelope,
  type TurnSendOptions,
} from '@pierre/shared-types';
import { refusalReason } from './apiError';
import { COACH_SHARING_REVOKED } from './coachPlanHooks';
import { describeTurnFailure } from './turnFailure';

/**
 * Where a client keeps the switch's position: `localStorage` on web,
 * `AsyncStorage` on the phone — either satisfies this as it is. A read or a
 * write that throws or rejects is absorbed: the switch then starts at
 * Everyone, or keeps the choice for this session only.
 */
export interface AudienceStore {
  getItem(key: string): string | null | Promise<string | null>;
  setItem(key: string, value: string): void | Promise<void>;
}

/** What the private lane is built over, bound once by each client. */
export interface PrivateLaneApis {
  chatApi: Pick<ChatApi, 'openAthleteThread' | 'getConversationMessages' | 'sendTurn'>;
  audienceStore: AudienceStore;
}

/** What `usePrivateLane` reads from the room the client has open. */
export interface UsePrivateLaneInput {
  /** The room's group as `GET /api/groups/{id}` answers the viewer; `null` while it loads. */
  group: Pick<CoachingGroup, 'id' | 'coach_user_id' | 'solo_athlete'> | null;
  /** The signed-in user; `null` while unknown. */
  viewerId: string | null;
  /**
   * The conversations the client holds. The coach's work thread about the
   * athlete is found here; none is opened just to show its rows.
   */
  conversations: readonly Conversation[];
}

/**
 * Everything a private send accepts beyond the text: the turn's own options
 * and callbacks, which reach `chatApi.sendTurn` as the client passed them.
 * `onDone` and `onError` run after the lane has written the turn's rows (or
 * taken the pending row back), in the same tick, so a client clearing its
 * streaming bubble there never shows the reply twice.
 */
export interface PrivateSendOptions extends TurnSendOptions, TurnCallbacks {
  /** Aborts the turn's request, as `sendTurn`'s own signal does. */
  signal?: AbortSignal;
}

/**
 * What a private send came to. It never rejects.
 *
 * - `sent`: the turn finished; its rows are in the lane.
 * - `failed`: opening the work thread or the turn failed. `refused` is true
 *   when the server answered before any turn ran, so nothing was stored and
 *   the client may put the words back in the composer; otherwise a re-read
 *   of the lane brings whatever the server kept.
 * - `skipped`: nothing was sent — no lane in this room, blank text, or a
 *   private send already running.
 */
export type PrivateSendOutcome =
  | { kind: 'sent'; conversationId: string; turn: TurnEnvelope }
  | { kind: 'failed'; conversationId: string | null; error: Error; refused: boolean }
  | { kind: 'skipped' };

/** What `usePrivateLane` hands the room screen. */
export interface UsePrivateLaneResult {
  /** The room's one athlete while the lane exists: the viewer is its coach, and the room holds one athlete. */
  athlete: SoloAthlete | null;
  /** Whether the composer shows the Everyone / Only <agent> switch. */
  available: boolean;
  /** Who the next send reaches; always `room` while the lane does not exist. */
  audience: MessageAudience;
  /** Move the switch, remembered on this device for this viewer in this room. */
  setAudience: (audience: MessageAudience) => void;
  toggleAudience: () => void;
  /**
   * The coach's work thread about the athlete, once it exists — the id
   * `usePlanDraftCard`, `chatApi.stopTurn` and feedback calls on a private row
   * take. `null` until the conversation list holds it or the first private
   * send opens it.
   */
  conversationId: string | null;
  /**
   * The work thread's rows to draw, oldest first, tool plumbing dropped:
   * `composeRoomThread`'s third argument. Empty while the lane does not exist.
   */
  rows: Message[];
  isLoading: boolean;
  isError: boolean;
  /** The server is still answering a private question no send here is streaming (a reload mid-turn). */
  awaitingReply: boolean;
  /** The athlete's room message "Adjust plan from this" quoted, until it is sent or removed. */
  quote: RoomQuote | null;
  /** Whether "Adjust plan from this" shows on `row`: the athlete's own readable room words. */
  canAdjustFrom: (row: QuotableRow) => boolean;
  /** "Adjust plan from this": quote `row` and switch to the agent alone. False when `row` cannot be quoted. */
  adjustFrom: (row: QuotableRow) => boolean;
  removeQuote: () => void;
  /** Send `content` to the work thread, carrying the quote; opens the thread first when it does not exist yet. */
  send: (content: string, options?: PrivateSendOptions) => Promise<PrivateSendOutcome>;
  isSending: boolean;
}

/** How often the lane is read again while the server says a private reply is still being written. */
const AWAITING_REPLY_POLL_MS = 4_000;

/** The id prefix of the question row the lane shows until the server's copy replaces it. */
const PENDING_ROW_PREFIX = 'private-pending-';

/**
 * The room's one athlete when `viewerId` is the group's human coach and the
 * server named one — which it does only for that coach, while one athlete
 * remains. `null` for every other reader and room.
 */
export function soloAthleteFor(
  group: Pick<CoachingGroup, 'coach_user_id' | 'solo_athlete'> | null,
  viewerId: string | null,
): SoloAthlete | null {
  if (!group?.solo_athlete || viewerId === null || group.coach_user_id !== viewerId) return null;
  return group.solo_athlete;
}

/**
 * The coach's work thread about `athleteUserId` in `groupId` (carnet#1070):
 * bound to the athlete through that group, and to no room of its own.
 */
export function findWorkThread(
  conversations: readonly Conversation[],
  athleteUserId: string,
  groupId: string,
): Conversation | null {
  return (
    conversations.find(
      (conversation) =>
        conversation.subject_user_id === athleteUserId &&
        conversation.subject_group_id === groupId &&
        !conversation.group_id,
    ) ?? null
  );
}

/** The storage key the switch's position is kept under, per viewer and room. */
export function audienceStorageKey(viewerId: string, groupId: string): string {
  return `dravr.privateLane.audience:${viewerId}:${groupId}`;
}

/** The remembered position, or Everyone when nothing readable is stored. */
async function readAudience(store: AudienceStore, key: string): Promise<MessageAudience> {
  try {
    return (await store.getItem(key)) === 'private' ? 'private' : 'room';
  } catch {
    return 'room';
  }
}

/** Remember the position; a store that refuses leaves it held for this session only. */
function writeAudience(store: AudienceStore, key: string, audience: MessageAudience): void {
  try {
    void Promise.resolve(store.setItem(key, audience)).catch(() => undefined);
  } catch {
    // A store that throws on write (a private window's storage) keeps nothing;
    // the choice still holds in the cache for as long as the app runs.
  }
}

/** Whether `error` refused the quote itself — the quoted message can no longer be used. */
function isQuoteRefusal(error: unknown): boolean {
  return isRoomQuoteRefusalReason(refusalReason(error));
}

/**
 * The sentence a failed private send shows. A refused quote says the message
 * can no longer be used for the plan; an athlete who stopped sharing with the
 * coach is said by name; everything else is worded as any failed turn is.
 */
export function privateSendFailureText(
  error: unknown,
  context: { t: Translate; athleteName: string; online?: boolean },
): string {
  const { t, athleteName, online } = context;
  if (isQuoteRefusal(error)) return t(PRIVATE_LANE_KEYS.quoteUnavailable);
  if (refusalReason(error) === COACH_SHARING_REVOKED) {
    return t(PLAN_DRAFT_KEYS.consentRevoked, { name: athleteName });
  }
  return describeTurnFailure(error, { t, online });
}

/** Each row once, the first occurrence kept. */
function uniqueById(rows: readonly Message[]): Message[] {
  const seen = new Set<string>();
  return rows.filter((row) => {
    if (seen.has(row.id)) return false;
    seen.add(row.id);
    return true;
  });
}

/**
 * The lane's rows once `turn` landed: the coach's question as the server
 * wrote it — under its real id, with the quote the server copied — in place
 * of the pending row, then the reply with its scenes, then any welcome the
 * turn wrote. An empty echo keeps the question as it was shown.
 */
function settledRows(rows: readonly Message[], pending: Message, turn: TurnEnvelope): Message[] {
  const echoed = turn.user_message;
  const question = echoed?.id && echoed.content ? echoed : pending;
  const reply = turn.assistant.message;
  const added: Message[] = [];
  if (reply.id && reply.content) {
    added.push({ ...reply, role: reply.role ?? 'assistant', scene_blocks: replySceneBlocks(turn) });
  }
  if (turn.welcome_message) added.push(turn.welcome_message);
  return uniqueById([...rows.map((row) => (row.id === pending.id ? question : row)), ...added]);
}

/** The thrown value as the `Error` a turn's `onError` receives. */
function asError(error: unknown): Error {
  return error instanceof Error ? error : new TurnFailedError('The work thread could not be opened.', null);
}

/**
 * Build the private-lane hook over one client's chat API and storage.
 *
 * Each client binds this once (`hooks/usePrivateLane`), passing its chat API
 * and its storage. Call `usePrivateLane` once per open room and hand the
 * result down: the quote lives in it.
 */
export function createPrivateLaneHooks({ chatApi, audienceStore }: PrivateLaneApis) {
  /**
   * A coach's private lane in a one-athlete room (carnet#1249).
   *
   * The switch shows only to the group's coach while the server names the
   * room's one athlete. "Only <agent>" turns go to the coach's work thread
   * about that athlete — never to the room — and its rows are read here for
   * the room to draw inline; nobody else reads them. The work thread is found
   * in the client's conversation list and opened only by the first private
   * send, so looking at a room never creates one.
   */
  function usePrivateLane({ group, viewerId, conversations }: UsePrivateLaneInput): UsePrivateLaneResult {
    const queryClient = useQueryClient();
    const athlete = soloAthleteFor(group, viewerId);
    const groupId = athlete !== null ? (group?.id ?? null) : null;
    const athleteId = athlete?.user_id ?? null;
    const available = athleteId !== null && groupId !== null && viewerId !== null;

    const [sending, setSending] = useState(false);
    const sendingRef = useRef(false);
    // Held with the room and athlete they belong to, so a room switched to
    // without a remount never inherits another room's quote or thread.
    const [heldQuote, setHeldQuote] = useState<{ groupId: string; quote: RoomQuote } | null>(null);
    const [opened, setOpened] = useState<{ groupId: string; athleteId: string; conversationId: string } | null>(
      null,
    );

    const audienceKey = QUERY_KEYS.chat.privateLaneAudience(viewerId, groupId);
    const storedAudience = useQuery({
      queryKey: audienceKey,
      queryFn: async () => {
        const stored = await readAudience(audienceStore, audienceStorageKey(viewerId as string, groupId as string));
        // A move made while the store was being read is the coach's latest word.
        return queryClient.getQueryData<MessageAudience>(audienceKey) ?? stored;
      },
      enabled: available,
      staleTime: Number.POSITIVE_INFINITY,
      retry: false,
    });
    const audience: MessageAudience = available ? (storedAudience.data ?? 'room') : 'room';

    const found = useMemo(
      () => (athleteId !== null && groupId !== null ? findWorkThread(conversations, athleteId, groupId) : null),
      [conversations, athleteId, groupId],
    );
    const openedId =
      available && opened?.groupId === groupId && opened.athleteId === athleteId ? opened.conversationId : null;
    const conversationId = openedId ?? found?.id ?? null;
    // An archived thread is brought back by opening it, as a thread not yet made is made.
    const opensFirst = openedId === null && (found === null || Boolean(found.archived_at));

    const messages = useQuery<MessagesResponse>({
      queryKey: QUERY_KEYS.chat.messages(conversationId),
      queryFn: () => chatApi.getConversationMessages(conversationId as string),
      // Never while a send here runs: it writes the turn's rows itself, and a
      // read racing it would drop the pending question.
      enabled: conversationId !== null && !sending,
      refetchOnWindowFocus: true,
      refetchInterval: (query) => (query.state.data?.awaiting_reply ? AWAITING_REPLY_POLL_MS : false),
    });
    const data = conversationId !== null ? messages.data : undefined;
    const rows = useMemo(() => filterDisplayMessages(data?.messages ?? []), [data]);

    const quote =
      available && heldQuote?.groupId === groupId && heldQuote.quote.author_user_id === athleteId
        ? heldQuote.quote
        : null;

    const updateAudience = useCallback(
      (next: (current: MessageAudience) => MessageAudience) => {
        if (!available || viewerId === null || groupId === null) return;
        const key = QUERY_KEYS.chat.privateLaneAudience(viewerId, groupId);
        const value = next(queryClient.getQueryData<MessageAudience>(key) ?? 'room');
        queryClient.setQueryData<MessageAudience>(key, value);
        writeAudience(audienceStore, audienceStorageKey(viewerId, groupId), value);
      },
      [available, viewerId, groupId, queryClient],
    );
    const setAudience = useCallback((value: MessageAudience) => updateAudience(() => value), [updateAudience]);
    const toggleAudience = useCallback(
      () => updateAudience((current) => (current === 'private' ? 'room' : 'private')),
      [updateAudience],
    );

    const canAdjustFrom = useCallback(
      (row: QuotableRow) => available && athleteId !== null && roomQuoteOf(row, athleteId) !== null,
      [athleteId, available],
    );
    const adjustFrom = useCallback(
      (row: QuotableRow) => {
        if (!available || athleteId === null || groupId === null) return false;
        const quoted = roomQuoteOf(row, athleteId);
        if (quoted === null) return false;
        setHeldQuote({ groupId, quote: quoted });
        setAudience('private');
        return true;
      },
      [available, athleteId, groupId, setAudience],
    );
    const removeQuote = useCallback(() => setHeldQuote(null), []);
    /** Drop the quote `sent` named, leaving one the coach picked since. */
    const releaseQuote = useCallback((sent: RoomQuote) => {
      setHeldQuote((held) => (held?.quote.entry_id === sent.entry_id ? null : held));
    }, []);

    const send = useCallback(
      async (content: string, options: PrivateSendOptions = {}): Promise<PrivateSendOutcome> => {
        if (!available || athleteId === null || groupId === null || content.trim().length === 0 || sendingRef.current) {
          return { kind: 'skipped' };
        }
        sendingRef.current = true;
        setSending(true);
        const sentQuote = quote;
        try {
          let threadId = conversationId;
          if (opensFirst || threadId === null) {
            try {
              const thread = await chatApi.openAthleteThread(athleteId, groupId);
              threadId = thread.id;
              setOpened({ groupId, athleteId, conversationId: thread.id });
              void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
            } catch (thrown) {
              const error = asError(thrown);
              options.onError?.(error);
              return { kind: 'failed', conversationId: null, error, refused: true };
            }
          }
          const thread = threadId;
          const key = QUERY_KEYS.chat.messages(thread);
          await queryClient.cancelQueries({ queryKey: key });
          const pending: Message = {
            id: `${PENDING_ROW_PREFIX}${Date.now()}`,
            conversation_id: thread,
            role: 'user',
            content: options.display ?? content,
            created_at: new Date().toISOString(),
            ...(sentQuote ? { quote: sentQuote } : {}),
          };
          queryClient.setQueryData<MessagesResponse>(key, (old) => ({
            ...old,
            messages: [...(old?.messages ?? []), pending],
          }));

          const result: { outcome: PrivateSendOutcome | null } = { outcome: null };
          await chatApi.sendTurn(thread, content, {
            signal: options.signal,
            origin: options.origin,
            ...(sentQuote ? { quoteEntryId: sentQuote.entry_id } : {}),
            onProgress: options.onProgress,
            onDelta: options.onDelta,
            onRetract: options.onRetract,
            onBlock: options.onBlock,
            onDone: (turn) => {
              queryClient.setQueryData<MessagesResponse>(key, (old) => ({
                ...old,
                messages: settledRows(old?.messages ?? [], pending, turn),
                awaiting_reply: null,
              }));
              if (sentQuote) releaseQuote(sentQuote);
              result.outcome = { kind: 'sent', conversationId: thread, turn };
              options.onDone?.(turn);
            },
            onError: (error) => {
              queryClient.setQueryData<MessagesResponse>(key, (old) =>
                old ? { ...old, messages: old.messages.filter((row) => row.id !== pending.id) } : old,
              );
              // A refused quote cannot be sent again; any other failure keeps it for the retry.
              if (sentQuote && isQuoteRefusal(error)) releaseQuote(sentQuote);
              result.outcome = {
                kind: 'failed',
                conversationId: thread,
                error,
                refused: error instanceof TurnRequestError,
              };
              options.onError?.(error);
            },
          });

          // The list row moves and previews the turn; the lane is read again
          // once this send lets it, so it holds what the server kept.
          void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
          void queryClient.invalidateQueries({ queryKey: key });
          return (
            result.outcome ?? {
              kind: 'failed',
              conversationId: thread,
              error: new TurnFailedError('The turn ended without an outcome.', null),
              refused: false,
            }
          );
        } finally {
          sendingRef.current = false;
          setSending(false);
        }
      },
      [available, athleteId, groupId, quote, conversationId, opensFirst, queryClient, releaseQuote],
    );

    return {
      athlete: available ? athlete : null,
      available,
      audience,
      setAudience,
      toggleAudience,
      conversationId,
      rows: conversationId !== null ? rows : [],
      isLoading: conversationId !== null && messages.isLoading,
      isError: conversationId !== null && messages.isError,
      awaitingReply: !sending && Boolean(data?.awaiting_reply),
      quote,
      canAdjustFrom,
      adjustFrom,
      removeQuote,
      send,
      isSending: sending,
    };
  }

  return { usePrivateLane };
}
