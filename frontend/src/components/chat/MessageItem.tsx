// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Individual message item in the chat message list — one switch over the turn's reply blocks
// ABOUTME: The server decided what this surface draws; nothing here re-derives it from the reply prose

import { memo, useEffect, useMemo, useState, type ReactNode } from 'react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Copy, Share2, ThumbsUp, ThumbsDown, RefreshCw, ShieldAlert } from 'lucide-react';
import type { ChatMessageAction, ClaimVerdict, ReplyBlock } from '@pierre/shared-types';
import {
  mergeVerdictSeverities,
  parseWorkoutPlan,
  summarizeVerdicts,
  verdictSummaryLabel,
  type VerdictTone,
} from '@pierre/shared-types';
import type { Message, MessageMetadata, MessageFeedback } from './types';
import { countActivities, parseSceneBlocks, splitVizMarkers, transcriptBlocks } from '@pierre/chat-utils';
import { linkifyUrls } from './utils';
import { SceneView } from './SceneView';
import WorkoutPlanCard from './WorkoutPlanCard';
import { MARKDOWN_COMPONENTS } from './markdownComponents';
import { useTranslation } from '@pierre/i18n';

interface MessageItemProps {
  message: Message;
  metadata?: MessageMetadata;
  feedback?: MessageFeedback;
  isError?: boolean;
  /**
   * What the server decided this surface draws for this turn, in its order.
   *
   * Present on the turn that just landed — the pipeline read the web surface's
   * render capabilities and produced this list. A history row has no block list
   * on the wire, so one is decoded from the persisted row instead; either way
   * the switch below walks exactly one list.
   */
  blocks?: ReplyBlock[];
  /** Claim verdicts attached to this message, if any. */
  verdicts?: ClaimVerdict[];
  /** Label shown above assistant turns — the active coach's name, or
   *  'Dravr' when the conversation has no coach attached. */
  assistantLabel?: string;
  onCopy?: () => void;
  onShare?: () => void;
  onThumbsUp?: () => void;
  onThumbsDown?: () => void;
  /** Saved thumbs-down reason for this message, hydrated on reload. */
  feedbackComment?: string;
  /** Persist an optional "what went wrong?" reason for a thumbs-down. */
  onSubmitReason?: (comment: string) => void;
  onRetry?: () => void;
  /** Open the verdict detail drawer for a single verdict. */
  onShowVerdict?: (verdict: ClaimVerdict) => void;
  /** Send a follow-up user message (used by "ask me about this claim"). */
  onAskAboutClaim?: (verdict: ClaimVerdict) => void;
  /** Press handler for a control the reply's `actions` block carried. */
  onActionClick?: (action: ChatMessageAction) => void;
}

/** Chip classes for the tone the shared rollup assigned the worst verdict. */
function chipClassForTone(tone: VerdictTone): string {
  switch (tone) {
    case 'success':
      return 'bg-success/15 text-on-success-container hover:bg-success/25';
    case 'warning':
      return 'bg-warning/15 text-on-warning-container hover:bg-warning/25';
    case 'error':
      return 'bg-error/15 text-error hover:bg-error/25';
    case 'info':
      return 'bg-info/15 text-on-info-container hover:bg-info/25';
    case 'secondary':
    default:
      return 'bg-surface-container-high/15 text-on-surface hover:bg-surface-container-high/25';
  }
}

/**
 * Optional "what went wrong?" reason captured after a thumbs-down. Submitting
 * is optional — the down rating is already persisted; this only adds a comment
 * to the same feedback row. Pre-fills with any saved reason on reload.
 */
function FeedbackReasonForm({
  initialComment,
  onSubmit,
}: {
  initialComment?: string;
  onSubmit: (comment: string) => void;
}) {
  const { t } = useTranslation();
  const [value, setValue] = useState(initialComment ?? '');
  const [saved, setSaved] = useState(false);

  // The saved reason can arrive after mount (conversation reload); sync it in
  // as long as the user hasn't started editing.
  useEffect(() => {
    setValue(initialComment ?? '');
  }, [initialComment]);

  const submit = () => {
    onSubmit(value.trim());
    setSaved(true);
  };

  return (
    <div className="mt-2 flex items-center gap-2">
      <input
        type="text"
        value={value}
        onChange={(e) => {
          setValue(e.target.value);
          setSaved(false);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') submit();
        }}
        placeholder={t('chat.feedbackReasonPlaceholder')}
        aria-label={t('chat.feedbackReasonLabel')}
        className="flex-1 max-w-xs px-2 py-1 text-xs rounded bg-surface-container-high text-on-surface placeholder:text-outline border border-outline/20 focus:outline-none focus:border-primary"
      />
      <button
        type="button"
        onClick={submit}
        className="px-2 py-1 text-xs rounded bg-primary/15 text-primary hover:bg-primary/25 transition-colors"
      >
        {saved ? t('chat.feedbackReasonSaved') : t('chat.feedbackReasonSend')}
      </button>
    </div>
  );
}

const MessageItem = memo(function MessageItem({
  message,
  metadata,
  feedback,
  isError = false,
  blocks,
  verdicts,
  assistantLabel,
  onCopy,
  onShare,
  onThumbsUp,
  onThumbsDown,
  feedbackComment,
  onSubmitReason,
  onRetry,
  onShowVerdict,
  onAskAboutClaim,
  onActionClick,
}: MessageItemProps) {
  const { t } = useTranslation();
  const isUser = message.role === 'user';

  const messageVerdicts = useMemo(
    () => (verdicts ?? []).filter((v) => v.message_id === message.id),
    [verdicts, message.id],
  );

  // One list, whatever the turn's age: the server's own blocks when it just
  // landed, the same shape decoded from the persisted row when it did not.
  const replyBlocks = useMemo(
    () => blocks ?? transcriptBlocks(message, messageVerdicts),
    [blocks, message, messageVerdicts],
  );

  // The reply's charts, which the prose positions with its ⟦viz:N⟧ markers
  // rather than appending in a lump — so they are resolved once here and drawn
  // inside the prose arm below.
  const scenes = useMemo(() => {
    const scene = replyBlocks.find((block) => block.type === 'scene');
    return scene?.type === 'scene' ? parseSceneBlocks(scene.scene_blocks) : [];
  }, [replyBlocks]);

  /**
   * Draw one reply block.
   *
   * The only place this surface decides what a piece of a reply looks like.
   * Every arm renders what the server already chose to send; none of them
   * inspects the prose to work out whether a different arm should have fired.
   */
  function renderBlock(block: ReplyBlock, key: number): ReactNode {
    switch (block.type) {
      case 'prose': {
        const segments = splitVizMarkers(linkifyUrls(block.text));
        return (
          <div
            key={key}
            className={`text-on-surface text-sm leading-relaxed prose prose-sm dark:prose-invert max-w-none prose-a:text-primary prose-a:underline hover:prose-a:text-primary/80 ${isError ? 'text-error' : ''}`}
          >
            {segments.map((segment, i) =>
              segment.kind === 'prose' ? (
                <Markdown key={i} remarkPlugins={[remarkGfm]} components={MARKDOWN_COMPONENTS}>
                  {segment.text}
                </Markdown>
              ) : (
                scenes[segment.index] && <SceneView key={i} block={scenes[segment.index]} />
              ),
            )}
          </div>
        );
      }

      case 'activity_list':
        return (
          <details key={key} className="mb-3">
            <summary className="cursor-pointer text-sm text-on-surface-variant hover:text-on-surface transition-colors select-none">
              Your Activities ({countActivities(block.text)})
            </summary>
            <div className="mt-2 ml-4 text-on-surface text-sm prose prose-sm dark:prose-invert max-w-none">
              <Markdown remarkPlugins={[remarkGfm]} components={MARKDOWN_COMPONENTS}>
                {block.text}
              </Markdown>
            </div>
          </details>
        );

      case 'workout_plan': {
        const plan = parseWorkoutPlan(JSON.stringify(block.plan));
        return plan ? <WorkoutPlanCard key={key} plan={plan} /> : null;
      }

      // The charts this block carries are positioned by the prose's own
      // ⟦viz:N⟧ markers and drawn in the prose arm. Drawing them here too is
      // the same chart twice.
      case 'scene':
        return null;

      case 'scene_image':
        return (
          <img
            key={key}
            src={block.url}
            alt={block.caption ?? t('chat.chartAlt')}
            className="mt-3 max-w-full rounded-lg"
          />
        );

      case 'verdicts': {
        const summary = summarizeVerdicts(mergeVerdictSeverities(messageVerdicts, block.chips));
        if (!summary || isUser) return null;
        const detail = messageVerdicts[0];
        return (
          <div key={key} className="mt-2 flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={detail && onShowVerdict ? () => onShowVerdict(detail) : undefined}
              className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs transition-colors ${chipClassForTone(summary.tone)}`}
              title={t('app.claimVerdictSummary', { count: summary.count, status: summary.worstStatus, strength: summary.worstStrength ?? summary.worstStatus })}
            >
              <ShieldAlert className="w-3 h-3" />
              <span>{verdictSummaryLabel(summary)}</span>
            </button>
            {summary.count > 1 ? <span className="text-xs text-outline">{t('chat.clickForDetails')}</span> : null}
            {onAskAboutClaim && detail && summary.count === 1 ? (
              <button
                type="button"
                onClick={() => onAskAboutClaim(detail)}
                className="text-xs text-primary hover:underline"
              >
                {t('chat.askAboutClaim')}
              </button>
            ) : null}
          </div>
        );
      }

      case 'actions':
        return (
          <div key={key} className="mt-3">
            {block.title ? (
              <div className="mb-1.5 text-xs font-medium text-on-surface-variant">{block.title}</div>
            ) : null}
            <div className="flex flex-wrap gap-2">
              {block.actions.map((action, idx) => (
                <button
                  key={`${action.value}-${idx}`}
                  type="button"
                  onClick={() => onActionClick?.(action)}
                  className="inline-flex items-center px-3 py-1.5 rounded-lg text-sm bg-primary/15 text-primary hover:bg-primary/25 transition-colors"
                >
                  {action.label}
                </button>
              ))}
            </div>
          </div>
        );

      case 'reconnect':
        return (
          <div key={key} className="mt-3">
            <a
              href={block.url}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium bg-primary text-on-primary hover:bg-primary/90 transition-colors no-underline"
            >
              <RefreshCw className="w-4 h-4" />
              {t('frag.reconnect')} {block.display_name}
            </a>
          </div>
        );

      // A quota notice is a fact about the turn, not about the message: the
      // conversation's usage banner shows it once, above the transcript.
      case 'notice':
        return null;
    }
  }

  return (
    <div className="flex gap-3">
      {/* Avatar */}
      <div className="flex-shrink-0">
        {isUser ? (
          <div className="w-8 h-8 rounded-full bg-surface-container-high flex items-center justify-center">
            <svg className="w-4 h-4 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" />
            </svg>
          </div>
        ) : (
          <img src="/dravr-icon.svg" alt={assistantLabel ?? t('shell.brandName')} className="w-8 h-8 rounded-xl" />
        )}
      </div>
      {/* Message Content */}
      <div className="flex-1 min-w-0 pt-1">
        <div className="font-medium text-on-surface text-sm mb-1">
          {isUser ? 'You' : (assistantLabel ?? t('shell.brandName'))}
        </div>
        {replyBlocks.map((block, index) => renderBlock(block, index))}
        {/* Action icons and metadata for assistant messages - matches mobile design */}
        {!isUser && (
          <div className="mt-2 flex items-center gap-4">
            {isError ? (
              /* For error messages, show only Retry button with label */
              <button
                onClick={onRetry}
                className="flex items-center gap-1 px-2 py-1 text-xs text-on-surface font-medium bg-surface-container-high rounded hover:bg-white/15 transition-colors"
              >
                <RefreshCw className="w-3.5 h-3.5" />
                <span>{t('chat.retry')}</span>
              </button>
            ) : (
              /* Normal assistant message actions */
              <>
                {/* Copy */}
                {onCopy && (
                  <button
                    onClick={onCopy}
                    className="p-0.5 text-outline hover:text-on-surface transition-colors"
                    title={t('chat.copyMessage')}
                  >
                    <Copy className="w-3.5 h-3.5" />
                  </button>
                )}
                {/* Share - always visible */}
                {onShare && (
                  <button
                    onClick={onShare}
                    className="p-0.5 text-outline hover:text-on-surface transition-colors"
                    title={t('chat.share')}
                  >
                    <Share2 className="w-3.5 h-3.5" />
                  </button>
                )}
                {/* Thumbs Up */}
                {onThumbsUp && (
                  <button
                    onClick={onThumbsUp}
                    className={`p-0.5 transition-colors ${
                      feedback === 'up' ? 'text-primary' : 'text-outline hover:text-on-surface'
                    }`}
                    title={t('chat.feedbackGood')}
                  >
                    <ThumbsUp className={`w-3.5 h-3.5 ${feedback === 'up' ? 'fill-current' : ''}`} />
                  </button>
                )}
                {/* Thumbs Down */}
                {onThumbsDown && (
                  <button
                    onClick={onThumbsDown}
                    className={`p-0.5 transition-colors ${
                      feedback === 'down' ? 'text-error' : 'text-outline hover:text-on-surface'
                    }`}
                    title={t('chat.feedbackPoor')}
                  >
                    <ThumbsDown className={`w-3.5 h-3.5 ${feedback === 'down' ? 'fill-current' : ''}`} />
                  </button>
                )}
                {/* Retry */}
                {onRetry && (
                  <button
                    onClick={onRetry}
                    className="p-0.5 text-outline hover:text-on-surface transition-colors"
                    title={t('chat.regenerateResponse')}
                  >
                    <RefreshCw className="w-3.5 h-3.5" />
                  </button>
                )}
                {/* Model and response time - to the right of icons */}
                {metadata && (
                  <span className="text-xs text-outline ml-2">
                    {metadata.model}{metadata.executionTimeMs ? ` · ${(metadata.executionTimeMs / 1000).toFixed(1)}s` : ''}
                  </span>
                )}
              </>
            )}
          </div>
        )}
        {/* Optional thumbs-down reason — the down rating is already saved; this
            adds/updates the free-text comment on the same feedback row. */}
        {!isUser && !isError && feedback === 'down' && onSubmitReason && (
          <FeedbackReasonForm initialComment={feedbackComment} onSubmit={onSubmitReason} />
        )}
      </div>
    </div>
  );
});

export default MessageItem;
