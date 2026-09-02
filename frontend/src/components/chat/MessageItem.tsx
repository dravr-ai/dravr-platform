// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One persisted message as a messenger bubble — one switch over the turn's reply blocks inside it
// ABOUTME: The server decided what this surface draws; nothing here re-derives it from the reply prose

import { memo, useEffect, useMemo, useState, type ReactNode } from 'react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Copy, Share2, ThumbsUp, ThumbsDown, RefreshCw, Shield, ShieldAlert } from 'lucide-react';
import type { ChatMessageAction, ClaimVerdict, ReplyBlock } from '@pierre/shared-types';
import {
  parseWorkoutPlan,
  summarizeVerdicts,
  verdictChipSeverity,
  verdictToneAlerts,
  type VerdictTone,
} from '@pierre/shared-types';
import type { Message, MessageMetadata, MessageFeedback } from './types';
import {
  COMMAND_FINISH_REASON,
  countActivities,
  parseSceneBlocks,
  splitVizMarkers,
  transcriptBlocks,
} from '@pierre/chat-utils';
import { EVIDENCE_STRENGTH_LABEL_KEY, VERDICT_STATUS_LABEL_KEY, verdictChipLabel } from '@pierre/shared-constants';
import { linkifyUrls } from './utils';
import { SceneView } from './SceneView';
import WorkoutPlanCard from './WorkoutPlanCard';
import MessageBubble from './MessageBubble';
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
  /** The clock inside the bubble, already formatted for the reader. */
  timestamp?: string;
  /** First row of a run by the same author — gets the avatar and the author line. */
  groupStart?: boolean;
  onCopy?: () => void;
  onShare?: () => void;
  onThumbsUp?: () => void;
  onThumbsDown?: () => void;
  /** Saved thumbs-down reason for this message, hydrated on reload. */
  feedbackComment?: string;
  /** Persist an optional "what went wrong?" reason for a thumbs-down. */
  onSubmitReason?: (comment: string) => void;
  onRetry?: () => void;
  /**
   * Open the verdict drawer for this message. Receives the rows the surface
   * has for it — possibly none yet, when only the turn's chips have landed,
   * in which case the host fetches them.
   */
  onShowVerdict?: (verdicts: ClaimVerdict[], messageId: string) => void;
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
      return 'bg-surface-container-high text-on-surface hover:bg-surface-container-highest';
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
        className="flex-1 max-w-xs rounded border ghost-border bg-surface-container-high px-2 py-1 text-xs text-on-surface placeholder:text-outline focus:border-primary focus:outline-none"
      />
      <button
        type="button"
        onClick={submit}
        className="rounded bg-primary/15 px-2 py-1 text-xs text-primary transition-colors hover:bg-primary/25"
      >
        {saved ? t('chat.feedbackReasonSaved') : t('chat.feedbackReasonSend')}
      </button>
    </div>
  );
}

/** The action-row icon buttons share one look; only the active feedback tones differ. */
const ACTION_BUTTON = 'p-0.5 text-outline transition-colors hover:text-on-surface';

const MessageItem = memo(function MessageItem({
  message,
  metadata,
  feedback,
  isError = false,
  blocks,
  verdicts,
  assistantLabel,
  timestamp,
  groupStart = true,
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
  const isCommand = message.finish_reason === COMMAND_FINISH_REASON;
  const author = assistantLabel ?? t('shell.brandName');

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
        // The athlete's own words are shown as typed — a question is not
        // markdown, and a stray asterisk should stay an asterisk.
        if (isUser) {
          return (
            <p key={key} className="whitespace-pre-wrap break-words text-sm leading-relaxed">
              {block.text}
            </p>
          );
        }
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
            <summary className="cursor-pointer select-none text-sm text-on-surface-variant transition-colors hover:text-on-surface">
              {t('app.yourActivitiesCount', { count: countActivities(block.text) })}
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
        // The turn's chips preview the verdicts until the rows land; once the
        // rows exist they are the count, so the chip cannot grow when a chip
        // and its row spell the claim differently.
        const severities =
          messageVerdicts.length > 0
            ? messageVerdicts.map((row) => ({ status: row.status, evidence_strength: row.evidence_strength }))
            : block.chips.map(verdictChipSeverity);
        const summary = summarizeVerdicts(severities);
        if (!summary || isUser) return null;
        const single = messageVerdicts.length === 1 ? messageVerdicts[0] : null;
        return (
          <div key={key} className="mt-2 flex flex-wrap items-center gap-2">
            <button
              type="button"
              data-testid="verdict-chip"
              onClick={onShowVerdict ? () => onShowVerdict(messageVerdicts, message.id) : undefined}
              className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs transition-colors focus-ring ${chipClassForTone(summary.tone)}`}
              title={t('app.claimVerdictSummary', {
                count: summary.count,
                status: t(VERDICT_STATUS_LABEL_KEY[summary.worstStatus]),
                strength: t(
                  summary.worstStrength === null
                    ? VERDICT_STATUS_LABEL_KEY[summary.worstStatus]
                    : EVIDENCE_STRENGTH_LABEL_KEY[summary.worstStrength],
                ),
              })}
            >
              {verdictToneAlerts(summary.tone) ? (
                <ShieldAlert className="h-3 w-3" aria-hidden="true" />
              ) : (
                <Shield className="h-3 w-3" aria-hidden="true" />
              )}
              <span>{verdictChipLabel(t, summary)}</span>
            </button>
            {summary.count > 1 ? <span className="text-xs text-outline">{t('chat.clickForDetails')}</span> : null}
            {onAskAboutClaim && single ? (
              <button
                type="button"
                onClick={() => onAskAboutClaim(single)}
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
                  className="inline-flex items-center rounded-lg bg-primary/15 px-3 py-1.5 text-sm text-primary transition-colors hover:bg-primary/25"
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
              className="inline-flex items-center gap-2 rounded-lg bg-primary px-4 py-2 text-sm font-medium text-on-primary no-underline transition-colors hover:bg-primary/90"
            >
              <RefreshCw className="h-4 w-4" />
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

  if (isUser) {
    return (
      <MessageBubble side="user" timestamp={timestamp} groupStart={groupStart}>
        <span className="sr-only">{t('chat.senderYou')}</span>
        {replyBlocks.map((block, index) => renderBlock(block, index))}
      </MessageBubble>
    );
  }

  // The row's actions live under the bubble and show on hover, focus or a
  // coarse pointer. An error row offers only the retry; a command reply, only
  // the copy — there is no model to rate or re-run.
  const actions = isError ? (
    onRetry && (
      <button
        onClick={onRetry}
        className="flex items-center gap-1 rounded bg-surface-container-high px-2 py-1 text-xs font-medium text-on-surface transition-colors hover:bg-surface-container-highest"
      >
        <RefreshCw className="h-3.5 w-3.5" />
        <span>{t('chat.retry')}</span>
      </button>
    )
  ) : (
    <div role="group" aria-label={t('chat.messageActions')} className="flex items-center gap-3">
      {onCopy && (
        <button onClick={onCopy} className={ACTION_BUTTON} title={t('chat.copyMessage')}>
          <Copy className="h-3.5 w-3.5" />
        </button>
      )}
      {onShare && !isCommand && (
        <button onClick={onShare} className={ACTION_BUTTON} title={t('chat.share')}>
          <Share2 className="h-3.5 w-3.5" />
        </button>
      )}
      {onThumbsUp && !isCommand && (
        <button
          onClick={onThumbsUp}
          className={`p-0.5 transition-colors ${feedback === 'up' ? 'text-primary' : 'text-outline hover:text-on-surface'}`}
          title={t('chat.feedbackGood')}
        >
          <ThumbsUp className={`h-3.5 w-3.5 ${feedback === 'up' ? 'fill-current' : ''}`} />
        </button>
      )}
      {onThumbsDown && !isCommand && (
        <button
          onClick={onThumbsDown}
          className={`p-0.5 transition-colors ${feedback === 'down' ? 'text-error' : 'text-outline hover:text-on-surface'}`}
          title={t('chat.feedbackPoor')}
        >
          <ThumbsDown className={`h-3.5 w-3.5 ${feedback === 'down' ? 'fill-current' : ''}`} />
        </button>
      )}
      {onRetry && !isCommand && (
        <button onClick={onRetry} className={ACTION_BUTTON} title={t('chat.regenerateResponse')}>
          <RefreshCw className="h-3.5 w-3.5" />
        </button>
      )}
      {metadata && !isCommand && (
        <span className="ml-2 text-xs text-outline">
          {metadata.model}{metadata.executionTimeMs ? ` · ${(metadata.executionTimeMs / 1000).toFixed(1)}s` : ''}
        </span>
      )}
    </div>
  );

  return (
    <>
      <MessageBubble
        side="assistant"
        authorLabel={author}
        avatar={<img src="/dravr-icon.svg" alt={author} className="h-8 w-8 rounded-full" />}
        timestamp={timestamp}
        groupStart={groupStart}
        finishReason={message.finish_reason}
        actions={actions}
      >
        {replyBlocks.map((block, index) => renderBlock(block, index))}
      </MessageBubble>
      {/* Optional thumbs-down reason — the down rating is already saved; this
          adds/updates the free-text comment on the same feedback row. */}
      {!isError && feedback === 'down' && onSubmitReason && (
        <div className="ml-10">
          <FeedbackReasonForm initialComment={feedbackComment} onSubmit={onSubmitReason} />
        </div>
      )}
    </>
  );
});

export default MessageItem;
