// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The thread's transcript — bubbles grouped by author, a day pill between days, and the live rows of a turn
// ABOUTME: Streaming text, the thinking dots, an error and a provider notice all speak from the coach's side of the thread

import { useRef, useEffect, useMemo, type ReactNode } from 'react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {
  dayLabelFor,
  filterDisplayMessages,
  formatMessageTime,
  isSameMessageGroup,
  localDayKey,
} from '@pierre/chat-utils';
import MessageItem from './MessageItem';
import MessageBubble from './MessageBubble';
import DaySeparator from './DaySeparator';
import type { ChatMessageAction, ClaimVerdict, ReplyBlock } from '@pierre/shared-types';
import type { Message, MessageMetadata, MessageFeedback, OAuthNotification } from './types';
import { linkifyUrls } from './utils';
import { MARKDOWN_COMPONENTS } from './markdownComponents';
import { useTranslation } from '@pierre/i18n';

interface MessageListProps {
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
  /** Label shown above assistant turns — the active coach's name, or
   *  'Dravr' when the conversation has no coach attached. */
  assistantLabel?: string;
  isLoading: boolean;
  isStreaming: boolean;
  streamingContent: string;
  /** Live AG-UI pipeline status for the in-flight turn (e.g. "calling
   *  get_activities…"), or `null` when no progress is known. Rendered
   *  in the streaming bubble alongside the token-delta text. */
  progressStatusText?: string | null;
  errorMessage: string | null;
  oauthNotification: OAuthNotification | null;
  onDismissError: () => void;
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
  /** "Ask me about this claim" callback → ChatTab dispatches a follow-up. */
  onAskAboutClaim?: (verdict: ClaimVerdict) => void;
  /** Press handler for a control the reply's `actions` block carried. */
  onActionClick?: (action: ChatMessageAction) => void;
}

/** The coach's mark beside a live row — the same one a persisted reply carries. */
function CoachAvatar({ label }: { label: string }) {
  return <img src="/dravr-icon.svg" alt={label} className="h-8 w-8 rounded-full" />;
}

export default function MessageList({
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
  progressStatusText,
  errorMessage,
  oauthNotification,
  onDismissError,
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
}: MessageListProps) {
  const { t, language } = useTranslation();
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const author = assistantLabel ?? t('shell.brandName');

  // Auto-scroll to bottom when messages change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, streamingContent]);

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
    let previousDay = '';
    for (const msg of visibleMessages) {
      const day = localDayKey(msg.created_at);
      if (day && day !== previousDay) {
        const label = dayLabelFor(msg.created_at, language);
        out.push(
          <DaySeparator
            key={`day-${day}`}
            label={
              label.kind === 'today'
                ? t('chat.dayToday')
                : label.kind === 'yesterday'
                  ? t('chat.dayYesterday')
                  : label.label
            }
          />,
        );
        previousDay = day;
      }
      const groupStart = previous === null || !isSameMessageGroup(previous, msg);
      out.push(
        <MessageItem
          key={msg.id}
          message={msg}
          metadata={messageMetadata.get(msg.id)}
          feedback={messageFeedback.get(msg.id)}
          feedbackComment={messageFeedbackComment.get(msg.id)}
          isError={msg.isError}
          blocks={messageBlocks?.get(msg.id)}
          verdicts={verdicts}
          assistantLabel={assistantLabel}
          timestamp={formatMessageTime(msg.created_at, language)}
          groupStart={groupStart}
          // The bubble hands back the reply as a reader outside the app can
          // use it — its charts named rather than left as ⟦viz:N⟧ markers —
          // because only the bubble has the resolved scenes to name them from.
          onCopy={msg.role === 'assistant' ? onCopyMessage : undefined}
          onShare={msg.role === 'assistant' ? onShareMessage : undefined}
          onThumbsUp={msg.role === 'assistant' ? () => onThumbsUp(msg.id) : undefined}
          onThumbsDown={msg.role === 'assistant' ? () => onThumbsDown(msg.id) : undefined}
          onSubmitReason={
            msg.role === 'assistant' ? (comment: string) => onSubmitFeedbackReason(msg.id, comment) : undefined
          }
          onRetry={msg.role === 'assistant' ? () => onRetryMessage(msg.id) : undefined}
          onShowVerdict={onShowVerdict}
          onAskAboutClaim={onAskAboutClaim}
          onActionClick={onActionClick}
        />,
      );
      previous = msg;
    }
    return out;
  }, [
    visibleMessages,
    language,
    t,
    messageMetadata,
    messageFeedback,
    messageFeedbackComment,
    messageBlocks,
    verdicts,
    assistantLabel,
    onCopyMessage,
    onShareMessage,
    onThumbsUp,
    onThumbsDown,
    onSubmitFeedbackReason,
    onRetryMessage,
    onShowVerdict,
    onAskAboutClaim,
    onActionClick,
  ]);

  if (isLoading) {
    return (
      <div className="py-8 text-center text-sm text-on-surface-variant">{t('chat.loadingMessages')}</div>
    );
  }

  return (
    <div className="flex flex-col">
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

      {/* Streaming response */}
      {isStreaming && streamingContent && (
        <MessageBubble side="assistant" authorLabel={author} avatar={<CoachAvatar label={author} />}>
          <div className="text-on-surface text-sm leading-relaxed prose prose-sm dark:prose-invert max-w-none prose-a:text-primary prose-a:underline hover:prose-a:text-primary/80">
            <Markdown remarkPlugins={[remarkGfm]} components={MARKDOWN_COMPONENTS}>
              {linkifyUrls(streamingContent)}
            </Markdown>
          </div>
          {/* Live pipeline progress (tool calls / steps) below the streamed
              text so the user sees what the model is doing even after the
              first tokens arrive. */}
          {progressStatusText && (
            <div className="mt-1 flex items-center gap-2 text-xs text-on-surface-variant">
              <div className="pierre-spinner h-3 w-3"></div>
              <span>{progressStatusText}</span>
            </div>
          )}
        </MessageBubble>
      )}

      {/* Thinking indicator — three breathing dots, the way every messenger says "typing" */}
      {isStreaming && !streamingContent && (
        <MessageBubble side="assistant" authorLabel={author} avatar={<CoachAvatar label={author} />}>
          <div className="flex items-center gap-2 text-sm text-on-surface-variant" role="status" aria-live="polite">
            <span className="flex items-center gap-1" aria-hidden="true">
              <span className="ai-typing-dot" />
              <span className="ai-typing-dot" />
              <span className="ai-typing-dot" />
            </span>
            <span>{progressStatusText ?? t('chat.thinking')}</span>
          </div>
        </MessageBubble>
      )}

      {/* Error message display */}
      {errorMessage && !isStreaming && (
        <MessageBubble side="assistant" authorLabel={author} avatar={<CoachAvatar label={author} />}>
          <div className="rounded-lg border border-error/30 bg-error/10 px-3 py-2">
            <p className="text-sm text-error">{errorMessage}</p>
            <button
              onClick={onDismissError}
              className="mt-2 text-xs text-error underline transition-colors hover:text-error"
            >
              {t('chat.dismiss')}
            </button>
          </div>
        </MessageBubble>
      )}

      <div ref={messagesEndRef} />
    </div>
  );
}
