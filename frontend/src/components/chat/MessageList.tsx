// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The thread's transcript — bubbles grouped by author, a day pill between days, and the live rows of a turn
// ABOUTME: Streaming text, a running turn's steps, an error and a provider notice all speak from the agent's side of the thread

import { useRef, useEffect, useLayoutEffect, useMemo, type ReactNode } from 'react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {
  audienceOf,
  dayLabelFor,
  filterDisplayMessages,
  formatMessageTime,
  isDivider,
  isSameMessageGroup,
  localDayKey,
  messageChannelOrigin,
  opensPrivateStretch,
  PRIVATE_LANE_KEYS,
  type TurnTrail,
} from '@pierre/chat-utils';
import MessageItem from './MessageItem';
import MessageBubble from './MessageBubble';
import DaySeparator from './DaySeparator';
import TopicDivider from './TopicDivider';
import CoachAvatar from './CoachAvatar';
import { FoldedTurnTrail, LiveTurnIndicator, LiveTurnLine } from './TurnTrail';
import { PrivateLaneDivider, PrivateReplyTag } from './PrivateLane';
import type { ChatMessageAction, ClaimVerdict, MessageAudience, ReplyBlock } from '@pierre/shared-types';
import type { Message, MessageMetadata, MessageFeedback, OAuthNotification } from './types';
import { linkifyUrls, topicAnchorId } from './utils';
import { MARKDOWN_COMPONENTS } from './markdownComponents';
import { useTranslation } from '@pierre/i18n';

/**
 * What the transcript needs of a coach's private lane in a one-athlete room
 * (carnet#1249). Absent in every other thread.
 */
export interface TranscriptPrivateLane {
  /** The coach's work thread the private rows come from; a draft card on one acts on it. */
  conversationId: string | null;
  /** Whether "Adjust plan from this" shows on a room row: the athlete's own readable words. */
  canAdjustFrom: (row: Message) => boolean;
  /** "Adjust plan from this": quote the row and turn the composer to the agent alone. */
  onAdjustFrom: (row: Message) => void;
  /** A control on a private reply — answered in the private lane, never in the room. */
  onActionClick?: (action: ChatMessageAction) => void;
  /** "Ask me about this claim" on a private reply — drafted for the private lane. */
  onAskAboutClaim?: (verdict: ClaimVerdict) => void;
}

interface MessageListProps {
  /**
   * The open thread. A plan card in a coach's plan thread acts on the
   * thread's draft through it; `null` before a thread exists.
   */
  conversationId?: string | null;
  messages: Message[];
  messageMetadata: Map<string, MessageMetadata>;
  messageFeedback: Map<string, MessageFeedback>;
  /** Saved thumbs-down reasons, keyed by assistant message id. */
  messageFeedbackComment: Map<string, string>;
  /**
   * The server's own reply blocks, keyed by assistant message id.
   *
   * Filled by the turn that produced them: the pipeline read this surface's
   * render capabilities and decided which pieces get their own block. Not
   * persisted — a message with no entry is drawn from its transcript row.
   */
  messageBlocks?: Map<string, ReplyBlock[]>;
  /** Claim verdicts for the active conversation, keyed by message_id. */
  verdicts?: ClaimVerdict[];
  /** Label shown above assistant turns — the active agent's name, or
   *  'Dravr' when the conversation has no agent attached. */
  assistantLabel?: string;
  isLoading: boolean;
  isStreaming: boolean;
  streamingContent: string;
  /**
   * The steps of the turn streaming here, or `null` when none streams. Drawn
   * as a checked list with the current step and a timer where the reply will
   * be, and as one line under the reply's words once they stream.
   */
  liveTrail?: TurnTrail | null;
  /**
   * When the reply the server is still writing was asked for (epoch ms),
   * with no stream here — the thread was reloaded mid-turn. The indicator
   * shows from then, with no steps, since they are not known.
   */
  awaitingSince?: number | null;
  /** Each finished reply's trail this session, keyed by the reply's id; folded above it. */
  turnTrails?: ReadonlyMap<string, TurnTrail>;
  errorMessage: string | null;
  oauthNotification: OAuthNotification | null;
  onDismissError: () => void;
  /** Re-send the question the failed turn left in the thread; absent when there is none to re-send. */
  onRetryError?: () => void;
  onDismissOAuthNotification: () => void;
  onCopyMessage: (content: string) => void;
  onShareMessage: (content: string) => void;
  onThumbsUp: (messageId: string) => void;
  onThumbsDown: (messageId: string) => void;
  /** Persist an optional thumbs-down reason for a message. */
  onSubmitFeedbackReason: (messageId: string, comment: string) => void;
  onRetryMessage: (messageId: string) => void;
  /** Click handler for the verdict chip → open the drawer for that message. */
  onShowVerdict?: (verdicts: ClaimVerdict[], messageId: string) => void;
  /** "Ask me about this claim" callback → ChatThread drafts a follow-up. */
  onAskAboutClaim?: (verdict: ClaimVerdict) => void;
  /** Press handler for a control the reply's `actions` block carried. */
  onActionClick?: (action: ChatMessageAction) => void;
  /**
   * Whether opening a thread brings its newest row into view. The chat
   * surface does (default); a thread embedded under something the athlete
   * came to read — an activity's figures — opens where it is, and follows
   * only the turns that arrive after.
   */
  followOnOpen?: boolean;
  /**
   * Older rows remain before the first one drawn: the thread is read in
   * windows from the newest back, and reaching its top reads the next one.
   */
  hasEarlier?: boolean;
  /** The window before the first row drawn is being read. */
  loadingEarlier?: boolean;
  /** The reader reached the top of what is drawn while older rows remain. */
  onReachStart?: () => void;
  /** A coach's private lane in this room; absent everywhere else. */
  privateLane?: TranscriptPrivateLane | null;
  /**
   * Who the turn streaming or awaited here speaks to. `private` draws its
   * live rows as the private lane's, under its divider when they open a
   * private stretch.
   */
  liveAudience?: MessageAudience;
}

/** The nearest ancestor that scrolls, which holds the thread's position. */
function scrollParent(node: HTMLElement | null): HTMLElement | null {
  let current = node?.parentElement ?? null;
  while (current) {
    const { overflowY } = getComputedStyle(current);
    if (overflowY === 'auto' || overflowY === 'scroll') return current;
    current = current.parentElement;
  }
  return null;
}

export default function MessageList({
  conversationId = null,
  messages,
  messageMetadata,
  messageFeedback,
  messageFeedbackComment,
  messageBlocks,
  verdicts,
  assistantLabel,
  isLoading,
  isStreaming,
  streamingContent,
  liveTrail = null,
  awaitingSince = null,
  turnTrails,
  errorMessage,
  oauthNotification,
  onDismissError,
  onRetryError,
  onDismissOAuthNotification,
  onCopyMessage,
  onShareMessage,
  onThumbsUp,
  onThumbsDown,
  onSubmitFeedbackReason,
  onRetryMessage,
  onShowVerdict,
  onAskAboutClaim,
  onActionClick,
  followOnOpen = true,
  hasEarlier = false,
  loadingEarlier = false,
  onReachStart,
  privateLane = null,
  liveAudience = 'room',
}: MessageListProps) {
  const { t, language } = useTranslation();
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const startRef = useRef<HTMLDivElement>(null);
  const author = assistantLabel ?? t('shell.brandName');
  const firstMessageId = messages[0]?.id;
  const lastMessageId = messages[messages.length - 1]?.id;

  // Reaching the top of what is drawn reads the window before it. The
  // sentinel sits above the first row, so it shows only once the reader has
  // scrolled back through everything held.
  useEffect(() => {
    const start = startRef.current;
    if (!start || !hasEarlier || loadingEarlier || !onReachStart) return;
    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) onReachStart();
    });
    observer.observe(start);
    return () => observer.disconnect();
  }, [hasEarlier, loadingEarlier, onReachStart, firstMessageId]);

  // A window read back lands above the rows the reader is looking at: the
  // view stays on them rather than jumping by the height of what arrived.
  const anchorRef = useRef<{ firstId?: string; lastId?: string; height: number }>({ height: 0 });
  useLayoutEffect(() => {
    const scroller = scrollParent(rootRef.current);
    const previous = anchorRef.current;
    if (
      scroller &&
      previous.firstId &&
      firstMessageId !== previous.firstId &&
      lastMessageId === previous.lastId
    ) {
      scroller.scrollTop += scroller.scrollHeight - previous.height;
    }
    anchorRef.current = {
      firstId: firstMessageId,
      lastId: lastMessageId,
      height: scroller?.scrollHeight ?? 0,
    };
  });

  // Auto-scroll to bottom when a row lands at the end. The first rows a
  // thread paints are its opening: a thread read back, which `followOnOpen`
  // may leave where it is, or the athlete's own first question, always
  // followed. A window read back lands at the top and is not followed.
  // The opening jump is instant: a smooth scroll would leave the top of the
  // window in view for its first frames, and the reader would be taken for
  // having scrolled back to it.
  const openedRef = useRef(false);
  useEffect(() => {
    if (!openedRef.current) {
      if (!lastMessageId) return;
      openedRef.current = true;
      if (!followOnOpen && !isStreaming) return;
      messagesEndRef.current?.scrollIntoView({ behavior: 'auto' });
      return;
    }
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [lastMessageId, streamingContent, followOnOpen, isStreaming]);

  // Filter out internal LLM plumbing rows (tool_call / tool_result) so their
  // raw <tool_call>/<tool_result> XML never renders — this matters most for
  // messaging-origin conversations (Telegram etc.) that surface in web chat
  // with the same scaffolding rows as native web chat.
  const visibleMessages = useMemo(() => filterDisplayMessages(messages), [messages]);

  // The rows with what surrounds them decided: a day pill where the local
  // date changes, and a run boundary where the author or the five-minute
  // window does. Both read `created_at`, so an optimistic row (stamped when
  // it was typed) sits in the right run too.
  const rows = useMemo(() => {
    const out: ReactNode[] = [];
    let previous: Message | null = null;
    // The row drawn before, whatever broke the run since: a private stretch
    // opens after a room row or at the top, never again after a day pill.
    let previousShown: Message | null = null;
    let previousDay = '';
    for (const msg of visibleMessages) {
      const day = localDayKey(msg.created_at);
      if (day && day !== previousDay) {
        out.push(<DaySeparator key={`day-${day}`} day={dayLabelFor(msg.created_at, language)} />);
        previousDay = day;
      }
      // A new topic in the personal thread: a rule, and the next row starts
      // a fresh run whoever wrote it.
      if (isDivider(msg)) {
        out.push(
          <TopicDivider
            key={msg.id}
            anchorId={topicAnchorId(msg.id)}
            label={t('chat.newChat')}
            time={formatMessageTime(msg.created_at, language)}
          />,
        );
        previous = null;
        continue;
      }
      // A coach's private rows (carnet#1249) open under a line saying who sees them.
      const isPrivate = audienceOf(msg) === 'private';
      if (privateLane && opensPrivateStretch(previousShown, msg)) {
        out.push(<PrivateLaneDivider key={`private-${msg.id}`} agent={author} />);
      }
      previousShown = msg;
      const groupStart = previous === null || !isSameMessageGroup(previous, msg);
      // Copy, rating and retry act on a row of the caller's own conversation;
      // a row the group thread shows from the room is not one, so it has none.
      // A private reply lives in the coach's work thread: it is copied, but
      // its rating and retry belong to that thread, not to this one.
      const ownReply = msg.role === 'assistant' && !msg.room && !isPrivate;
      const copyable = ownReply || (isPrivate && msg.role === 'assistant');
      // "Adjust plan from this", on the room's athlete's own words.
      const adjustable = privateLane !== null && !isPrivate && privateLane.canAdjustFrom(msg);
      // The personal thread is shared with the athlete's messaging DM: their
      // words from a messaging app say where they came from.
      const origin = messageChannelOrigin(msg);
      const time = formatMessageTime(msg.created_at, language);
      const foldedTrail = turnTrails?.get(msg.id);
      out.push(
        <MessageItem
          key={msg.id}
          conversationId={isPrivate ? (privateLane?.conversationId ?? null) : conversationId}
          message={msg}
          metadata={messageMetadata.get(msg.id)}
          feedback={messageFeedback.get(msg.id)}
          feedbackComment={messageFeedbackComment.get(msg.id)}
          isError={msg.isError}
          blocks={messageBlocks?.get(msg.id)}
          verdicts={verdicts}
          assistantLabel={assistantLabel}
          timestamp={origin ? `${time} · ${t('chat.fromChannel', { channel: origin.label })}` : time}
          groupStart={groupStart}
          lead={foldedTrail ? <FoldedTurnTrail trail={foldedTrail} /> : undefined}
          // The bubble hands back the reply as a reader outside the app can
          // use it — its charts named rather than left as ⟦viz:N⟧ markers —
          // because only the bubble has the resolved scenes to name them from.
          onCopy={copyable ? onCopyMessage : undefined}
          onShare={ownReply ? onShareMessage : undefined}
          onThumbsUp={ownReply ? () => onThumbsUp(msg.id) : undefined}
          onThumbsDown={ownReply ? () => onThumbsDown(msg.id) : undefined}
          onSubmitReason={
            ownReply ? (comment: string) => onSubmitFeedbackReason(msg.id, comment) : undefined
          }
          onRetry={ownReply ? () => onRetryMessage(msg.id) : undefined}
          onShowVerdict={onShowVerdict}
          onAskAboutClaim={isPrivate ? (privateLane?.onAskAboutClaim ?? onAskAboutClaim) : onAskAboutClaim}
          onActionClick={isPrivate ? privateLane?.onActionClick : onActionClick}
          onAdjustPlan={adjustable ? () => privateLane?.onAdjustFrom(msg) : undefined}
        />,
      );
      previous = msg;
    }
    return out;
  }, [
    conversationId,
    visibleMessages,
    language,
    t,
    messageMetadata,
    messageFeedback,
    messageFeedbackComment,
    messageBlocks,
    verdicts,
    assistantLabel,
    turnTrails,
    onCopyMessage,
    onShareMessage,
    onThumbsUp,
    onThumbsDown,
    onSubmitFeedbackReason,
    onRetryMessage,
    onShowVerdict,
    onAskAboutClaim,
    onActionClick,
    privateLane,
    author,
  ]);

  // The live rows of a private turn draw as the private lane's: the dashed
  // rule, the "Only you" tag, and the divider when the thread so far ended on
  // a row the room reads.
  const livePrivate = liveAudience === 'private';
  const lastShown = visibleMessages[visibleMessages.length - 1] ?? null;
  const liveOpensStretch = livePrivate && opensPrivateStretch(lastShown, { audience: 'private' });
  const liveBubble = {
    side: 'assistant' as const,
    authorLabel: author,
    avatar: <CoachAvatar label={author} />,
    audience: liveAudience,
    tag: livePrivate ? <PrivateReplyTag /> : undefined,
    ariaLabel: livePrivate ? t(PRIVATE_LANE_KEYS.bubbleAria, { agent: author }) : undefined,
  };
  const liveShown = isStreaming || awaitingSince !== null;

  if (isLoading) {
    return (
      <div className="py-8 text-center text-sm text-on-surface-variant">{t('chat.loadingMessages')}</div>
    );
  }

  return (
    // The thread's one reading column: 720px like the composer under it.
    <div ref={rootRef} className="mx-auto flex w-full max-w-[720px] flex-col [overflow-anchor:none]">
      {hasEarlier ? (
        <div ref={startRef} data-testid="earlier-messages" className="flex justify-center py-3">
          {loadingEarlier ? (
            <div className="pierre-spinner h-4 w-4" role="status" aria-label={t('chat.loadingMessages')} />
          ) : null}
        </div>
      ) : null}
      {rows}

      {/* OAuth connection notification */}
      {oauthNotification && (
        <MessageBubble side="assistant" authorLabel={author} avatar={<CoachAvatar label={author} />}>
          <div className="flex items-start gap-3">
            <p className="text-sm leading-relaxed text-on-surface">
              {t('app.providerConnected', { provider: oauthNotification.provider })}
            </p>
            <button
              onClick={onDismissOAuthNotification}
              className="text-outline transition-colors hover:text-on-surface"
              aria-label={t('chat.dismiss')}
            >
              <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        </MessageBubble>
      )}

      {liveShown && liveOpensStretch ? <PrivateLaneDivider agent={author} /> : null}

      {/* Streaming response, with what the turn is on under its words */}
      {isStreaming && streamingContent && (
        <MessageBubble {...liveBubble}>
          <div className="text-on-surface text-base leading-relaxed prose prose-sm dark:prose-invert max-w-none prose-a:text-primary prose-a:underline hover:prose-a:text-primary/80">
            <Markdown remarkPlugins={[remarkGfm]} components={MARKDOWN_COMPONENTS}>
              {linkifyUrls(streamingContent)}
            </Markdown>
          </div>
          {liveTrail ? <LiveTurnLine trail={liveTrail} /> : null}
        </MessageBubble>
      )}

      {/* The running turn before its words: the steps it has taken, the one it
          is on, and how long it has run. After a reload the server's word
          that a reply is coming stands in for the stream, with no steps. */}
      {isStreaming && !streamingContent && (
        <MessageBubble {...liveBubble}>
          <LiveTurnIndicator trail={liveTrail} />
        </MessageBubble>
      )}
      {!isStreaming && awaitingSince !== null && (
        <MessageBubble {...liveBubble}>
          <LiveTurnIndicator trail={null} startedAt={awaitingSince} />
        </MessageBubble>
      )}

      {/* Error message display */}
      {errorMessage && !isStreaming && (
        <MessageBubble side="assistant" authorLabel={author} avatar={<CoachAvatar label={author} />}>
          <div className="w-fit max-w-full rounded-lg border border-error/30 bg-error/10 px-3 py-2" data-testid="turn-error-box">
            <p className="text-sm text-error" data-testid="turn-error">{errorMessage}</p>
            <div className="mt-2 flex gap-3">
              {onRetryError && (
                <button
                  onClick={onRetryError}
                  className="text-xs font-medium text-error underline transition-colors hover:text-error"
                >
                  {t('chat.retry')}
                </button>
              )}
              <button
                onClick={onDismissError}
                className="text-xs text-error underline transition-colors hover:text-error"
              >
                {t('chat.dismiss')}
              </button>
            </div>
          </div>
        </MessageBubble>
      )}

      <div ref={messagesEndRef} />
    </div>
  );
}
