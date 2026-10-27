// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One conversation's state on the web — its reads, the turn in flight, a failed turn's note, the composer and how a turn opens a thread
// ABOUTME: Every chat surface (Groups, Home, an activity's view) owns one and draws it with ChatThread; each host decides how a new thread is opened

import { useState, useEffect, useRef, useCallback, useMemo, useReducer } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  AWAITING_REPLY_POLL_MS,
  awaitedSince,
  COMMAND_FINISH_REASON,
  composeRoomThread,
  hasEarlierMessages,
  joinMessageWindows,
  PLAN_THREAD_KEYS,
  providerStatusLine,
  readLostTurn,
  reduceLostTurn,
  threadSubtitle,
  type TurnTrail,
} from '@pierre/chat-utils';
import { holdIdleWhileBusy, idleSignal, MENTION_PREFIX, trackAbsence } from '@pierre/shared-constants';
import {
  describeApiError,
  describeTurnFailure,
  isTurnFailureRetryable,
  streamedReplyCallbacks,
  useTurnTrail,
} from '@pierre/ui-logic';
import { replySceneBlocks, type MessagesResponse } from '@pierre/api-client';
import type { Agent, ClaimVerdict, Conversation, MessageOrigin, ReplyBlock, TurnSendOptions } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { chatApi, groupsApi, providersApi } from '../../services/api';
import { track } from '../../services/analytics';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { useUsageStatus } from '../../hooks/useUsageStatus';
import { cachedConversations, useConversationList } from '../../hooks/useConversationList';
import { useMarkConversationRead } from '../../hooks/useMarkConversationRead';
import { useEarlierMessages } from '../../hooks/useEarlierMessages';
import { useCoachInfo } from '../../hooks/useCoachInfo';
import { useGroup } from '../../hooks/useGroups';
import { useAuth } from '../../hooks/useAuth';
import { useErrorToast } from '../ui';
import { useMessageFeedback } from './useMessageFeedback';
import { useOAuthCompletion } from './useOAuthCompletion';
import { useReplyTabMark } from '../../hooks/useReplyTabMark';
import type { Message, MessageFeedback, MessageMetadata, OAuthNotification, PendingComposerAction } from './types';

/**
 * The id prefix of the user row appended to the transcript while a turn is in
 * flight. It has no server id, so the read marker leaves it out of the
 * "newest persisted message" it advances to.
 */
const OPTIMISTIC_USER_ID_PREFIX = 'user-';

/**
 * What a failed turn leaves on screen until its reply lands or another turn
 * starts: the worded failure, the athlete's own question, and whether a retry
 * is worth offering.
 */
interface FailedTurnNote {
  text: string;
  question: Message;
  /**
   * What the turn sent, which a retry sends again: a tapped suggestion's
   * bubble shows its label while the turn carries its postback (carnet#828).
   */
  sent: { content: string; options: TurnSendOptions };
  retryable: boolean;
}

/** How a new thread is started. */
export interface NewThreadOptions {
  /**
   * The thread exists to carry a line about to be sent, so the agent the
   * server binds posts no welcome over its answer.
   */
  athleteSpeaksFirst?: boolean;
}

/**
 * How a host opens a thread for a turn sent with none open, answering its id.
 * The Groups tab and an activity's view create one; Home opens a topic in the
 * athlete's personal thread.
 */
export type OpenThread = (request: { athlete_speaks_first?: boolean }) => Promise<{ id: string }>;

/** The newest row the server has persisted — the one the read marker follows. */
function latestPersistedMessageId(messages: Message[] | undefined): string | null {
  if (!messages) return null;
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (!messages[i].id.startsWith(OPTIMISTIC_USER_ID_PREFIX)) return messages[i].id;
  }
  return null;
}

export interface UseChatThreadOptions {
  /** The conversation on screen, or `null` before there is one. */
  conversationId: string | null;
  onSelectConversation: (id: string | null) => void;
  /** How this surface opens a thread when a turn needs one. */
  openThread: OpenThread;
  /** Text the host wants drafted or sent in the thread — the invite deep link, an activity's chips. */
  pendingComposerAction?: PendingComposerAction | null;
  /** Called once the action above has been drafted or dispatched. */
  onPendingComposerActionConsumed?: () => void;
}

/** The thread's own rows as read so far, for a host that reads back to a point in them (Home's topics). */
export interface ThreadHistory {
  /** Whether the newest window has been read. */
  loaded: boolean;
  /** The caller's own rows held, oldest first, the turn's optimistic row included. */
  ownMessages: Message[] | undefined;
  /** Every persisted row held. */
  heldRows: Message[];
  hasEarlier: boolean;
  earlierLoading: boolean;
  loadBefore: (held: Message[]) => Promise<void>;
}

export interface ChatThreadState {
  conversationId: string | null;
  /** The open thread's list row: its title, its agent and its group. */
  activeConversation: Conversation | null;
  /** The thread's agent as the athlete's own agent list spells it — localized. */
  activeCoach: Agent | null;
  /** What the thread's replies are signed with: the group's agent, else the thread's own. */
  activeCoachTitle: string | null;
  /** The thread's stored title — the list row's — else the agent about to be talked to. */
  title: string;
  /** The line under the title: a plan thread's athlete, the group badge, the agent's handle, or what the agent can see. */
  subtitle: string | null;
  /** What the agent can read, worded; `null` until the status call answers. */
  providerStatus: string | null;
  /** The athlete has no provider connected, and the status call said so. */
  showConnectBanner: boolean;
  usageStatus: ReturnType<typeof useUsageStatus>;

  // The transcript.
  threadMessages: Message[];
  messagesLoading: boolean;
  roomFailed: boolean;
  history: ThreadHistory;
  reachStart: () => void;
  messageMetadata: Map<string, MessageMetadata>;
  messageBlocks: Map<string, ReplyBlock[]>;
  messageFeedback: Map<string, MessageFeedback>;
  messageFeedbackComment: Map<string, string>;
  thumbsUp: (messageId: string) => void;
  thumbsDown: (messageId: string) => void;
  submitFeedbackReason: (messageId: string, comment: string) => Promise<void>;
  verdicts: ClaimVerdict[];
  verdictsFetching: boolean;
  refetchVerdicts: () => void;

  // The turn in flight.
  isStreaming: boolean;
  /**
   * A turn is running in this thread: streaming here, or — after a reload —
   * one the server says it is still answering. The composer's stop button
   * shows for the whole of it.
   */
  turnInFlight: boolean;
  streamingContent: string;
  /** The steps of the turn streaming here; `null` when none streams. */
  liveTrail: TurnTrail | null;
  /**
   * When the reply the server is still writing was asked for (epoch ms),
   * when nothing streams here — a reload mid-turn. `null` otherwise.
   */
  awaitingSince: number | null;
  /** Each finished reply's trail this session, keyed by the reply's id. */
  turnTrails: ReadonlyMap<string, TurnTrail>;
  isStopping: boolean;
  stopTurn: () => Promise<void>;
  sendTurn: (content: string, options?: TurnSendOptions) => Promise<void>;
  retryMessage: (messageId: string) => Promise<void>;
  /** The error line under the transcript. */
  shownError: string | null;
  dismissError: () => void;
  /** Re-send a failed turn the server never stored; absent when no retry is worth offering. */
  retryFailedTurn: (() => void) | undefined;
  oauthNotification: OAuthNotification | null;
  dismissOAuthNotification: () => void;

  // The composer.
  composerText: string;
  setComposerText: (text: string) => void;
  /** Send what the composer holds and clear it. */
  sendComposer: () => void;
  /** Put a line the thread suggested in the composer, reported as a draft when sent. */
  seedDraft: (text: string) => void;
  /** Draft or send `text`, opening a thread first when none is open. */
  runComposerAction: (action: PendingComposerAction) => void;
  /** Open a new thread the host's way. */
  startThread: (options?: NewThreadOptions) => void;
  startingThread: boolean;
}

/**
 * The state of one conversation and every way a turn reaches it.
 *
 * The host owns it, so the host's own chrome — the Groups header and info
 * drawer, Home's Today and History, an activity's chips — reads and drives
 * the same thread the transcript draws.
 */
export function useChatThread({
  conversationId: selectedConversation,
  onSelectConversation,
  openThread,
  pendingComposerAction,
  onPendingComposerActionConsumed,
}: UseChatThreadOptions): ChatThreadState {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const showErrorToast = useErrorToast();
  const [newMessage, setNewMessage] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [streamingContent, setStreamingContent] = useState('');
  // A stop was sent for the in-flight turn and its stream has not ended yet.
  const [isStopping, setIsStopping] = useState(false);
  // The conversation the in-flight turn was posted to. The athlete can open
  // another thread while a turn streams, and a stop has to reach the turn that
  // is running, not the thread on screen.
  const streamingConversationRef = useRef<string | null>(null);
  // What the in-flight turn has done and is doing — the stage it entered,
  // the tools it called — read off the turn's own `progress` frames. Same
  // response body the reply arrives on, so there is nothing to correlate.
  const {
    live: liveTrail,
    finished: turnTrails,
    begin: beginTrail,
    progress: trailProgress,
    prose: trailProse,
    retract: trailRetract,
    finish: finishTrail,
  } = useTurnTrail();
  // A reply that lands while the tab is hidden marks the tab's title.
  const markReplyLanded = useReplyTabMark();
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  // A failed turn, kept by the reducer both clients share until a read of the
  // thread holds its reply or another turn starts. Web paints its note as the
  // error text under the transcript, and keeps the athlete's question so a
  // turn refused before the server stored it stays on screen with a retry.
  const [lostTurn, dispatchLostTurn] = useReducer(reduceLostTurn<FailedTurnNote>, null);
  const oauth = useOAuthCompletion(onSelectConversation);
  const [pendingPrompt, setPendingPrompt] = useState<Omit<PendingComposerAction, 'kind'> | null>(null);
  // Whether the composer holds text a draft affordance put there (Home Today,
  // the route draft, "back up this claim"), so its send is reported as a
  // draft rather than as typed. Cleared when the composer empties.
  const draftSeededRef = useRef(false);
  const [messageMetadata, setMessageMetadata] = useState<Map<string, MessageMetadata>>(new Map());
  // Which model wrote a reply, and how long it took, is an operator's reading
  // (carnet#1147): an athlete talks to their agent, never to a model id.
  const { user } = useAuth();
  const showsReplyModel = user?.role === 'admin' || user?.role === 'super_admin';
  // What the server decided this turn draws, keyed by assistant message id.
  // Not persisted: a reloaded conversation has no block list on the wire, so
  // its rows are decoded back into the same shape by the renderer.
  const [messageBlocks, setMessageBlocks] = useState<Map<string, ReplyBlock[]>>(new Map());

  // Fetch usage quota status for warning banners
  const usageStatus = useUsageStatus();

  // Fetch provider status (includes both OAuth and non-OAuth providers like synthetic)
  const { data: providersData, isSuccess: providersLoaded } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });

  const hasConnectedProvider = providersData?.providers?.some(p => p.connected) ?? false;
  // Gated on the query having actually answered, not on the `?? false` default.
  // While loading, `hasConnectedProvider` is false for everyone, so keying the
  // banner off it alone flashes a connect-provider nudge at users who are
  // connected — every chat load.
  const showConnectBanner = providersLoaded && !hasConnectedProvider;

  // Fetch messages for selected conversation — the caller's own rows. In a
  // group thread these are the model's context store, and the room below is
  // woven through them.
  const { data: messagesData, isLoading: messagesLoading, isPending: messagesPending } = useQuery<MessagesResponse>({
    queryKey: QUERY_KEYS.chat.messages(selectedConversation),
    queryFn: () => chatApi.getConversationMessages(selectedConversation!),
    enabled: !!selectedConversation,
    // Messaging turns arrive async via inbound webhook with no websocket push
    // to the web client. Refetch when the tab/window regains focus so a reply
    // sent from Telegram (or any channel) into an open conversation appears
    // without a manual reload. The same refetch is how a turn lost while the
    // athlete was away shows its reply: the idle watch's return is the focus
    // edge (`onActive` sets it), so the thread is re-read the moment they are
    // back, and the note below comes down once that read holds the answer.
    refetchOnWindowFocus: true,
    // A reload mid-turn holds no stream: the server's `awaiting_reply` says a
    // reply is still being written, and the thread is re-read every few
    // seconds until it lands. Never while a turn streams here — its own
    // `onDone` writes the reply, and a re-read racing it would drop rows.
    // Kept up in a hidden tab, which is where the athlete was told to go.
    refetchInterval: query =>
      !isStreaming && awaitedSince(query.state.data?.awaiting_reply, query.state.data?.messages ?? []) !== null
        ? AWAITING_REPLY_POLL_MS
        : false,
    refetchIntervalInBackground: true,
  });

  // The reply the server is still writing for a question in this thread,
  // when nothing streams here: the indicator shows from when it was asked.
  const serverAwaitingSince = useMemo(
    () => awaitedSince(messagesData?.awaiting_reply, messagesData?.messages ?? []),
    [messagesData],
  );
  const awaitingSince = isStreaming ? null : serverAwaitingSince;
  // A turn runs in this thread — streaming here, or restored after a reload.
  // Either way a second one must not start: sends wait behind it.
  const turnInFlight = isStreaming || awaitingSince !== null;
  // A queued send also waits for the thread's first read to answer: until it
  // does, nobody knows whether the server is still writing a reply in it.
  const holdQueuedSend = turnInFlight || messagesPending;

  // The open thread reads itself: the marker advances once the messages
  // resolve and again on every reply that lands, so the row's unread count
  // and the nav badge clear as the athlete reads.
  const latestMessageId = useMemo(
    () => latestPersistedMessageId(messagesData?.messages),
    [messagesData],
  );
  useMarkConversationRead(selectedConversation, latestMessageId);

  // A lost turn's note belongs to its own thread, and only for as long as the
  // reply has not landed: any read of the conversation that holds it — the
  // focus refetch on the athlete's return, reopening the thread — takes the
  // note down, and the reply renders from the transcript like any other row.
  const lostReading = useMemo(
    () => readLostTurn(lostTurn, selectedConversation, messagesData?.messages ?? []),
    [lostTurn, selectedConversation, messagesData],
  );
  // While the server says it is still writing the reply, the lost stream is
  // not a failure: the indicator stands in for the note until the reply lands
  // or the server stops waiting without one.
  const shownError = useMemo<string | null>(() => {
    if (!lostTurn || errorMessage !== lostTurn.note.text) return errorMessage;
    if (awaitingSince !== null) return null;
    return lostReading.kind === 'waiting' ? errorMessage : null;
  }, [errorMessage, lostTurn, lostReading, awaitingSince]);
  // The turn a shown failure can re-send: only a failure a retry may get
  // past (`isTurnFailureRetryable` — never the idle stop, a quota or a
  // refusal), and only a question the server never stored. One it did store
  // — the turn failed mid-stream — would be stored twice by a re-send.
  const retryableTurn =
    lostReading.kind === 'waiting' &&
    shownError === lostReading.note.text &&
    lostReading.note.retryable &&
    !lostReading.questionReceived
      ? lostReading.note.sent
      : null;

  const feedback = useMessageFeedback(selectedConversation, messagesData, setErrorMessage);

  // Claim verdicts attached to messages in the selected conversation.
  // Re-read when a turn completes, so a reply that was verified surfaces its
  // chip rail without a manual reload — the stream itself carries only the
  // flagged claims.
  const { data: verdictsData, isFetching: verdictsFetching, refetch } = useQuery({
    queryKey: QUERY_KEYS.chat.verdicts(selectedConversation),
    queryFn: () => chatApi.getConversationVerdicts(selectedConversation!),
    enabled: !!selectedConversation,
  });
  const verdicts: ClaimVerdict[] = useMemo(() => verdictsData?.verdicts ?? [], [verdictsData]);
  // Joins a read a completed turn already started instead of cancelling it.
  const refetchVerdicts = useCallback(() => {
    void refetch({ cancelRefetch: false });
  }, [refetch]);

  // The list is the one source for the open thread's row: its title, the
  // agent it is bound to and the group it is scoped to all come from there,
  // so the header and the info drawer read the same record the sidebar draws.
  const { conversations } = useConversationList();
  const activeConversation = useMemo(
    () => conversations.find(c => c.id === selectedConversation) ?? null,
    [conversations, selectedConversation],
  );

  const { coach: activeCoach } = useCoachInfo(activeConversation?.agent_id ?? null);
  // A group's agent may belong to another tenant, where the caller's own agent
  // list cannot see it; the group names its agent as the caller reads it.
  const { group: activeGroup } = useGroup(activeConversation?.group_id ?? '');
  const activeCoachTitle = activeGroup?.agent_title ?? activeCoach?.title ?? null;

  // A group thread renders the room, not only the caller's side of it: every
  // member's words and the agent's replies to them, each under its author, and
  // a placeholder wherever a member's sharing consent withholds an entry. The
  // room is read back to the oldest row of the caller's own conversation, so
  // no stretch of the thread shows one side only. It refetches on focus like
  // the messages do — another member's turn reaches no push either. One group
  // can hold several of the caller's threads (a channel session rotates), each
  // reaching back to its own oldest row, so the read is keyed by that reach:
  // a thread never paints a room read that stopped short of its history.
  const roomGroupId = activeConversation?.group_id ?? null;
  // The thread is read in windows from the newest back (carnet#840): the
  // windows read back on a scroll to the top sit before the newest one.
  const { earlier, loading: earlierLoading, loadBefore } = useEarlierMessages(selectedConversation);
  const ownMessages = useMemo(
    () => (messagesData ? joinMessageWindows(earlier?.messages, messagesData.messages) : undefined),
    [earlier, messagesData],
  );
  const hasEarlier = hasEarlierMessages(earlier, messagesData?.has_more);
  // Every persisted row held — an optimistic row is the turn's, not the thread's.
  const heldRows = useMemo(
    () => (ownMessages ?? []).filter(m => !m.id.startsWith(OPTIMISTIC_USER_ID_PREFIX)),
    [ownMessages],
  );
  const reachStart = useCallback(() => {
    if (hasEarlier && heldRows.length > 0) void loadBefore(heldRows);
  }, [hasEarlier, heldRows, loadBefore]);

  const roomSince = useMemo(
    () => ownMessages?.find(m => !m.id.startsWith(OPTIMISTIC_USER_ID_PREFIX))?.created_at ?? null,
    [ownMessages],
  );
  const { data: roomEntries, isError: roomFailed } = useQuery({
    queryKey: [...QUERY_KEYS.groups.room(roomGroupId ?? ''), roomSince],
    queryFn: () => groupsApi.readRoom(roomGroupId!, roomSince),
    enabled: !!roomGroupId && !!messagesData,
    refetchOnWindowFocus: true,
  });

  // The reply the server was writing, with nothing streaming here, has
  // landed — or the server stopped waiting without one. Whatever the reply
  // changed is re-read the way a live turn's `onDone` would: the list row,
  // the room's other lines, the verdict chips.
  const awaitedRef = useRef<{ conversation: string | null; since: number | null }>({
    conversation: null,
    since: null,
  });
  useEffect(() => {
    const previous = awaitedRef.current;
    awaitedRef.current = { conversation: selectedConversation, since: serverAwaitingSince };
    if (
      isStreaming ||
      !selectedConversation ||
      previous.conversation !== selectedConversation ||
      previous.since === null ||
      serverAwaitingSince !== null
    ) {
      return;
    }
    setIsStopping(false);
    const newest = messagesData?.messages[messagesData.messages.length - 1];
    if (newest?.role === 'assistant') markReplyLanded();
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.verdicts(selectedConversation) });
    if (roomGroupId) queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.room(roomGroupId) });
  }, [selectedConversation, serverAwaitingSince, isStreaming, messagesData, markReplyLanded, queryClient, roomGroupId]);
  // A failed turn's question the server never stored — the turn was refused
  // before it ran — is still the athlete's words: it stays in the thread, above
  // the failure note and its retry, rather than vanishing with the re-read.
  const ownWithQuestion = useMemo<Message[] | undefined>(
    () =>
      lostReading.kind === 'waiting' && !lostReading.questionReceived
        ? [...(ownMessages ?? []), lostReading.note.question]
        : ownMessages,
    [lostReading, ownMessages],
  );
  const threadMessages = useMemo<Message[]>(
    () =>
      roomGroupId && roomEntries
        ? composeRoomThread(ownWithQuestion ?? [], roomEntries)
        : ownWithQuestion ?? [],
    [roomGroupId, roomEntries, ownWithQuestion],
  );

  // What the thread is called: its stored title, which the server already
  // spells as the group, the agent, or the moment it started — the same
  // string the list row and the phone's header print. Before a thread
  // exists, the agent about to be talked to, else a new conversation.
  const title = useMemo<string>(() => {
    if (activeConversation) return activeConversation.title?.trim() || t('app.newConversation');
    return activeCoachTitle || t('app.newConversation');
  }, [activeConversation, activeCoachTitle, t]);

  // The line under the name: the agent's handle, or what the agent can see.
  const providerStatus = useMemo<string | null>(
    () => providerStatusLine(t, providersData?.providers, providersLoaded),
    [providersData, providersLoaded, t],
  );
  const subtitle = useMemo<string | null>(() => {
    const line = threadSubtitle(activeConversation);
    if (line?.kind === 'subject') return t(PLAN_THREAD_KEYS.subtitle, { name: line.name });
    if (line?.kind === 'group') return t('chat.groupChatBadge');
    if (line?.kind === 'handle') return `${MENTION_PREFIX}${line.handle}`;
    return providerStatus;
  }, [activeConversation, providerStatus, t]);

  // Mutations. The server resolves the agent's system prompt at runtime from
  // the coaches table, and binds the athlete's own agent when none is named.
  const createConversation = useMutation<{ id: string }, Error, NewThreadOptions | void>({
    // The host decides what a new thread is: Home opens a topic inside the
    // athlete's personal thread; anywhere else the server creates the thread
    // and names it — after its agent when one is attached, else for the
    // moment it starts in the viewer's language; a rename replaces that.
    mutationFn: (options) => openThread({ athlete_speaks_first: options?.athleteSpeaksFirst || undefined }),
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      onSelectConversation(data.id);
    },
    onError: (error) => {
      // The server caps active conversations (max_active_conversations) and
      // returns 429 QuotaExceeded once a user is at the limit. Without this the
      // agent click silently did nothing — surface a clear, actionable message.
      // Keyed on the limit the server named, not on the status: a spent
      // request budget is a 429 too, and it is not the conversation cap.
      const details = (error as {
        response?: { data?: { details?: { limit_type?: string; limit?: number } } };
      }).response?.data?.details;
      if (details?.limit_type === 'max_active_conversations') {
        showErrorToast(
          t('chat.conversationLimitTitle'),
          t('chat.conversationLimitBody', { limit: details.limit ?? '' })
        );
        return;
      }
      showErrorToast(
        t('app.couldNotStartChat'),
        describeApiError(error, { t, fallbackKey: 'app.conversationCreateFailed' })
      );
    },
  });

  /** Stable handle on the mutation above, so callbacks can depend on it. */
  const startConversation = createConversation.mutate;

  // Restore message metadata (model label) from conversation when loading existing messages
  useEffect(() => {
    if (!showsReplyModel || !selectedConversation || !messagesData?.messages?.length) return;
    const conversation = cachedConversations(queryClient).find(c => c.id === selectedConversation);
    if (!conversation?.model) return;

    setMessageMetadata(prev => {
      const newMap = new Map(prev);
      let changed = false;
      for (const msg of messagesData.messages) {
        if (msg.role === 'assistant' && !newMap.has(msg.id)) {
          newMap.set(msg.id, { model: conversation.model!, executionTimeMs: 0 });
          changed = true;
        }
      }
      return changed ? newMap : prev;
    });
  }, [showsReplyModel, selectedConversation, messagesData, queryClient]);

  /**
   * Send one turn and fold the result into the transcript.
   *
   * Takes its content as an argument rather than reading the composer, so
   * every caller — the send button, a queued agent prompt, a command action
   * button, a retry — dispatches the same way. The composer is state, so a
   * caller never seeds it and waits for React to commit before sending.
   */
  const sendTurn = useCallback(async (content: string, options: TurnSendOptions = {}) => {
    if (!content || !selectedConversation || turnInFlight) return;

    setIsStreaming(true);
    streamingConversationRef.current = selectedConversation;
    setStreamingContent('');
    setErrorMessage(null);
    dispatchLostTurn({ type: 'sent' });
    track({ name: 'feature_engaged', props: { feature: 'chat_message_sent' } });

    // What the thread held before this turn, so a re-read after a lost stream
    // can tell this turn's reply from the rows that were already there.
    const conversationKey = QUERY_KEYS.chat.messages(selectedConversation);
    const heldIds: ReadonlySet<string> = new Set(
      (queryClient.getQueryData<{ messages: Message[] }>(conversationKey)?.messages ?? []).map(m => m.id),
    );
    const leftDuringTurn = trackAbsence();

    const userMessageId = `${OPTIMISTIC_USER_ID_PREFIX}${Date.now()}`;
    const tempUserMessage: Message = {
      id: userMessageId,
      role: 'user',
      content: options.display ?? content,
      created_at: new Date().toISOString(),
    };

    // A read of the thread still in flight — a thread created a moment ago
    // for a question sent at once is still on its first — would land after
    // the row below and answer without it, wiping the athlete's own question
    // from the transcript. It is cancelled first; the turn's answer brings
    // what the thread holds.
    await queryClient.cancelQueries({ queryKey: conversationKey });
    queryClient.setQueryData(conversationKey, (old: MessagesResponse | undefined) => ({
      ...old,
      messages: [...(old?.messages || []), tempUserMessage],
    }));

    beginTrail();

    // The reply's words as they stream, and the trail's writing phase with them.
    const streamed = streamedReplyCallbacks(setStreamingContent, { prose: trailProse, retract: trailRetract });
    // The server decided which pieces this surface draws; `onBlock` collects
    // them in that order. Held here until `onDone` supplies the assistant
    // message id they are keyed by.
    const turnBlocks: ReplyBlock[] = [];
    // A streaming turn holds the client active: the athlete asked a question
    // and is watching for the answer, even with the mouse untouched. Released
    // in the `finally` below, so the idle threshold measures the quiet AFTER
    // the turn rather than racing the model.
    const releaseIdleHold = holdIdleWhileBusy();
    await chatApi.sendTurn(selectedConversation, content, {
      // A turn left streaming into a tab abandoned mid-answer still holds a
      // server instance open; the hold above covers the athlete who is
      // waiting, the signal covers the one who walked away.
      signal: idleSignal(),
      origin: options.origin,
      ...streamed,
      onProgress: progress => {
        // The shared trail names each step the same way here and on mobile.
        trailProgress(progress);
      },
      onBlock: block => {
        // A quota notice is a fact about the turn rather than about the reply,
        // and the conversation banner is where it belongs. Everything else is
        // part of the message and is drawn by the renderer's block switch.
        if (block.type === 'notice') {
          usageStatus.applyNotice(block.notice);
          return;
        }
        turnBlocks.push(block);
      },
      onDone: turn => {
        // The athlete's line as the server wrote it — a tapped suggestion's
        // words in place of the label shown while the turn ran, under the
        // row's real id so a re-read of the thread does not draw it twice. An
        // empty echo (a stale suggestion: nothing was written) keeps what
        // the athlete was shown.
        const echoed = turn.user_message;
        if (echoed?.content) {
          queryClient.setQueryData(conversationKey, (old: MessagesResponse | undefined) => ({
            ...old,
            messages: (old?.messages ?? []).map(m =>
              m.id === userMessageId
                ? { ...m, id: echoed.id, content: echoed.content, created_at: echoed.created_at }
                : m,
            ),
          }));
        }

        const assistantMessageId = turn.assistant.message.id;
        const { model, execution_time_ms: executionTimeMs } = turn.telemetry;

        // The trail folds into one line above the reply it led to, and a
        // reply landing in a hidden tab marks the tab.
        finishTrail(assistantMessageId || null, turn.telemetry.tools_called);
        markReplyLanded();

        if (showsReplyModel && assistantMessageId && model) {
          setMessageMetadata(prev => {
            const newMap = new Map(prev);
            newMap.set(assistantMessageId, { model, executionTimeMs });
            return newMap;
          });
        }

        // The panel, the controls, the chips and the reconnect call to action
        // all came from the block walk above; nothing here sniffs a flat field
        // or a sentence to work out what to draw.
        if (turnBlocks.length > 0 && assistantMessageId) {
          const blocks = [...turnBlocks];
          setMessageBlocks(prev => {
            const newMap = new Map(prev);
            newMap.set(assistantMessageId, blocks);
            return newMap;
          });
        }

        // Inject the persisted assistant message into the cache directly so
        // the streaming bubble can be cleared in the same batch — invalidating
        // and waiting for the refetch leaves a window where both the streaming
        // bubble (assembled deltas) and the just-arrived persisted message
        // render together, surfacing as a duplicated opening sentence. The
        // turn envelope carries the whole turn, so the assistant message it
        // holds already includes any post-processing additions the refetch
        // would have brought in.
        const assistantMessage = turn.assistant.message;
        if (assistantMessage.id && assistantMessage.content) {
          const persisted: Message = {
            id: assistantMessage.id,
            role: assistantMessage.role ?? 'assistant',
            content: assistantMessage.content,
            token_count: assistantMessage.token_count,
            created_at: assistantMessage.created_at ?? new Date().toISOString(),
            scene_blocks: replySceneBlocks(turn),
          };
          queryClient.setQueryData(
            QUERY_KEYS.chat.messages(selectedConversation),
            (old: MessagesResponse | undefined) => {
              const existing = old?.messages ?? [];
              // The turn is answered: a read taken while it ran said it was
              // awaited, and that must not outlive the reply.
              if (existing.some(m => m.id === persisted.id)) {
                return { ...old, messages: existing, awaiting_reply: null };
              }
              return { ...old, messages: [...existing, persisted], awaiting_reply: null };
            },
          );
        }

        // A command that bound an agent (`/agent add`) also wrote the agent's
        // welcome after its answer. The envelope carries it whole, starters
        // included, so it joins the cache the same way rather than waiting on
        // a re-read of the thread.
        const welcome = turn.welcome_message;
        if (welcome) {
          queryClient.setQueryData(
            QUERY_KEYS.chat.messages(selectedConversation),
            (old: MessagesResponse | undefined) => {
              const existing = old?.messages ?? [];
              if (existing.some(m => m.id === welcome.id)) {
                return { ...old, messages: existing };
              }
              return { ...old, messages: [...existing, welcome] };
            },
          );
        }

        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });

        // A turn is where a first-run flow is done — a reply, a tapped
        // starter, a grounded answer, a check-in answer — so Getting started
        // re-reads its rows after every one.
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.home.gettingStarted() });

        // The turn joined the room too, and so may have other members'
        // lines since the room was last read.
        if (roomGroupId) queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.room(roomGroupId) });

        // The turn's verdict rows are written before `done`, and the stream's
        // `verdicts` block names only the flagged claims — a reply whose claims
        // all held streams no chip. Re-reading the rows gives the live reply
        // the same rail a reload draws.
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.verdicts(selectedConversation) });

        // A slash command is the one turn that can rewrite what the header and
        // the info panel draw — `/agent add` binds an agent, `/agent remove`
        // detaches one, `/discover install` puts a new agent on the athlete's
        // list. Both read the agent set, so a command turn refreshes it; an
        // LLM turn never changes it and is left alone.
        if (turn.assistant.finish_reason === COMMAND_FINISH_REASON) {
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.list() });
        }

        // `/reset` archives this thread and continues on a fresh one;
        // `/group create` opens the new group's chat and files its answer
        // there. Selecting it is the whole switch: the selection drives the
        // hash, the messages query, the verdicts read and the read marker.
        // Last in the handler, because everything above is keyed on the
        // conversation the turn was posted to — including the cache write that
        // clears the streaming bubble — and the list invalidation above has
        // already brought the new row in. This turn's rows leave the thread
        // posted to: the server filed them there (`/reset`) or in the new one
        // (`/group create`), and the next read of this thread says which,
        // rather than a group chat showing another group's creation until then.
        const rotatedTo = turn.rotated_to_conversation_id;
        if (rotatedTo && rotatedTo !== selectedConversation) {
          const turnRows = new Set([userMessageId, echoed?.id, assistantMessageId]);
          queryClient.setQueryData(conversationKey, (old: MessagesResponse | undefined) =>
            old ? { ...old, messages: old.messages.filter(m => !turnRows.has(m.id)) } : old,
          );
          onSelectConversation(rotatedTo);
        }
        // `/reset` in the personal thread opens a topic instead: the athlete
        // stays, and the divider — written before this turn's own rows — is
        // drawn by re-reading the thread.
        if (turn.opened_topic_divider_id) {
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(selectedConversation) });
        }
      },
      onError: error => {
        finishTrail(null);
        // Worded from the shared catalogue by the failure's stable code, in the
        // athlete's language — never the server's English description.
        const note = describeTurnFailure(error, { t, online: navigator.onLine });
        setErrorMessage(note);
        queryClient.invalidateQueries({ queryKey: conversationKey });
        // A starter tap is recorded before the reply, so a failed reply can
        // still have finished a first-run flow.
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.home.gettingStarted() });
        if (roomGroupId) queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.room(roomGroupId) });
        // Failed while the athlete was away — the idle stop dropped the
        // stream, or the network went with a sleeping laptop. The server kept
        // going, so the note stands only until a read of the thread holds the
        // reply; the messages query re-reads it on their return.
        // Kept whether or not the athlete was away: the re-read below replaces
        // the transcript, and a turn refused before the server stored its
        // question would take the question with it.
        dispatchLostTurn({
          type: 'failed',
          away: leftDuringTurn(),
          keepWhenPresent: true,
          turn: {
            conversationId: selectedConversation,
            heldIds,
            note: {
              text: note,
              question: tempUserMessage,
              sent: { content, options },
              retryable: isTurnFailureRetryable(error, { online: navigator.onLine }),
            },
          },
        });
      },
    });

    releaseIdleHold();
    setIsStreaming(false);
    setIsStopping(false);
    streamingConversationRef.current = null;
    setStreamingContent('');
    // The delivered reply is now the source of truth. A turn that ended with
    // neither `onDone` nor `onError` has no trail left to show.
    finishTrail(null);
    usageStatus.invalidate();
  }, [selectedConversation, turnInFlight, queryClient, usageStatus, onSelectConversation, t, roomGroupId, showsReplyModel, beginTrail, trailProse, trailRetract, trailProgress, finishTrail, markReplyLanded]);

  /**
   * Stop the turn that is streaming.
   *
   * The server ends it and closes the question with a short notice; the open
   * stream then finishes through `onDone` like any other turn, so nothing is
   * torn down here. When the reply had already landed, or the stop could not
   * be sent, the button is handed back and the turn carries on.
   */
  const stopTurn = useCallback(async () => {
    // After a reload the turn runs with no stream here: the stop goes to the
    // thread the server says is still being answered, and a re-read brings
    // the notice it closes the question with.
    const restored = streamingConversationRef.current === null && awaitingSince !== null;
    const streamingConversation = streamingConversationRef.current ?? (restored ? selectedConversation : null);
    if (!streamingConversation || isStopping) return;
    setIsStopping(true);
    try {
      const stopped = await chatApi.stopTurn(streamingConversation);
      if (!stopped) setIsStopping(false);
      if (restored) {
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(streamingConversation) });
      }
    } catch (error) {
      setIsStopping(false);
      showErrorToast(
        t('chat.stopTurnAria'),
        describeApiError(error, { t, fallbackKey: 'chat.turnTryAgain' }),
      );
    }
  }, [isStopping, showErrorToast, t, awaitingSince, selectedConversation, queryClient]);

  /** The composer's own send: hand the typed text to {@link sendTurn} and clear the box. */
  const sendComposer = useCallback(() => {
    const typed = newMessage.trim();
    // While a turn runs the composer is disabled; a send that still reaches
    // here keeps the athlete's words in the box rather than dropping them.
    if (!typed || turnInFlight) return;
    const origin: MessageOrigin | undefined = draftSeededRef.current ? 'draft' : undefined;
    draftSeededRef.current = false;
    setNewMessage('');
    void sendTurn(typed, { origin });
  }, [newMessage, sendTurn, turnInFlight]);

  // A draft the athlete erased entirely is no longer a draft: what they type
  // next is their own.
  useEffect(() => {
    if (newMessage === '') draftSeededRef.current = false;
  }, [newMessage]);

  const seedDraft = useCallback((text: string) => {
    draftSeededRef.current = true;
    setNewMessage(text);
  }, []);

  // A prompt queued while the conversation was still being created, or while
  // a turn was running in it — streaming here or restored after a reload — is
  // sent as soon as there is a thread and no turn in it. It goes straight to
  // `sendTurn`, never through the composer and its send button.
  useEffect(() => {
    if (pendingPrompt && selectedConversation && !holdQueuedSend) {
      const promptToSend = pendingPrompt;
      setPendingPrompt(null);
      void sendTurn(promptToSend.text, { origin: promptToSend.origin, display: promptToSend.display });
    }
  }, [pendingPrompt, selectedConversation, holdQueuedSend, sendTurn]);

  /**
   * Put `text` in front of the athlete in a thread, creating one when none is
   * open. `send` dispatches it as a turn (the invite deep link, the "+" menu's
   * group creation, the info drawer's `/agent remove`); `draft` seeds the
   * composer and lets them finish the line.
   */
  const runComposerAction = useCallback((action: PendingComposerAction) => {
    if (action.kind === 'send' && selectedConversation) {
      // A turn already runs here, or the thread's first read has not said
      // whether one does: the line waits in the same queue a send with no
      // thread does, and goes out once that turn has its reply.
      if (holdQueuedSend) {
        setPendingPrompt({ text: action.text, origin: action.origin, display: action.display });
        return;
      }
      void sendTurn(action.text, { origin: action.origin, display: action.display });
      return;
    }
    if (action.kind === 'send') {
      // No thread yet: queue the text and let the pending-prompt effect send
      // it the moment the fresh conversation exists. The athlete speaks
      // first, so the thread's agent posts no welcome over the answer.
      setPendingPrompt({ text: action.text, origin: action.origin, display: action.display });
      startConversation({ athleteSpeaksFirst: true });
      return;
    }
    if (!selectedConversation) startConversation();
    // A command line (`/`, the palette opener) is not a question draft.
    draftSeededRef.current = !action.text.startsWith('/');
    setNewMessage(action.text);
  }, [selectedConversation, startConversation, sendTurn, holdQueuedSend]);

  // The host's own composer action — the `/groups/join/:code` deep link, which
  // lands on chat and sends `/group join CODE` as its first turn.
  useEffect(() => {
    if (!pendingComposerAction) return;
    runComposerAction(pendingComposerAction);
    onPendingComposerActionConsumed?.();
  }, [pendingComposerAction, runComposerAction, onPendingComposerActionConsumed]);

  /**
   * Retry a failed assistant turn.
   *
   * Drops the failed row from the transcript and re-sends the user message
   * that produced it, so the athlete is left with one attempt rather than a
   * failure followed by its replacement.
   */
  const retryMessage = useCallback(async (messageId: string) => {
    if (!selectedConversation || turnInFlight) return;

    const messages = messagesData?.messages || [];
    const messageIndex = messages.findIndex(m => m.id === messageId);
    if (messageIndex <= 0) return;

    // Walk back to the user message that produced this assistant turn.
    let userMessageIndex = messageIndex - 1;
    while (userMessageIndex >= 0 && messages[userMessageIndex].role !== 'user') {
      userMessageIndex--;
    }
    if (userMessageIndex < 0) return;

    queryClient.setQueryData(
      QUERY_KEYS.chat.messages(selectedConversation),
      (old: MessagesResponse | undefined) => ({
        ...old,
        messages: (old?.messages ?? []).filter(m => m.id !== messageId),
      }),
    );
    await sendTurn(messages[userMessageIndex].content);
  }, [selectedConversation, turnInFlight, messagesData?.messages, queryClient, sendTurn]);

  /** Re-send what a failed turn sent, shown as it was shown. */
  const retryFailedTurn = useCallback(() => {
    if (retryableTurn === null) return;
    void sendTurn(retryableTurn.content, retryableTurn.options);
  }, [retryableTurn, sendTurn]);

  const dismissError = useCallback(() => setErrorMessage(null), []);
  const startThread = useCallback((options?: NewThreadOptions) => startConversation(options), [startConversation]);

  return {
    conversationId: selectedConversation,
    activeConversation,
    activeCoach,
    activeCoachTitle,
    title,
    subtitle,
    providerStatus,
    showConnectBanner,
    usageStatus,
    threadMessages,
    messagesLoading,
    roomFailed,
    history: {
      loaded: !!messagesData,
      ownMessages,
      heldRows,
      hasEarlier,
      earlierLoading,
      loadBefore,
    },
    reachStart,
    messageMetadata,
    messageBlocks,
    messageFeedback: feedback.messageFeedback,
    messageFeedbackComment: feedback.messageFeedbackComment,
    thumbsUp: feedback.thumbsUp,
    thumbsDown: feedback.thumbsDown,
    submitFeedbackReason: feedback.submitReason,
    verdicts,
    verdictsFetching,
    refetchVerdicts,
    isStreaming,
    turnInFlight,
    streamingContent,
    liveTrail,
    awaitingSince,
    turnTrails,
    isStopping,
    stopTurn,
    sendTurn,
    retryMessage,
    shownError,
    dismissError,
    retryFailedTurn: retryableTurn !== null ? retryFailedTurn : undefined,
    oauthNotification: oauth.notification,
    dismissOAuthNotification: oauth.dismiss,
    composerText: newMessage,
    setComposerText: setNewMessage,
    sendComposer,
    seedDraft,
    runComposerAction,
    startThread,
    startingThread: createConversation.isPending,
  };
}
