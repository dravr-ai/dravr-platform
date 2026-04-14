// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Individual message item in the chat message list
// ABOUTME: Memoized for performance when rendering many messages

import { memo, useMemo } from 'react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Copy, Share2, Users, ThumbsUp, ThumbsDown, RefreshCw, Lightbulb, ShieldAlert } from 'lucide-react';
import type { ChatVerdictRow } from '@pierre/api-client';
import type { Message, MessageMetadata, MessageFeedback } from './types';
import { splitActivityContent, countActivities } from '@pierre/chat-utils';
import { linkifyUrls, stripContextPrefix } from './utils';

interface MessageItemProps {
  message: Message;
  metadata?: MessageMetadata;
  feedback?: MessageFeedback;
  isError?: boolean;
  hasInsight?: boolean;
  /** Pre-resolved activity list text (from API field or parsed from old content) */
  activityList?: string;
  /** Tier 5.5 claim verdicts attached to this message, if any. */
  verdicts?: ChatVerdictRow[];
  onCopy?: () => void;
  onShare?: () => void;
  onShareToFeed?: () => void;
  onCreateInsight?: () => void;
  onThumbsUp?: () => void;
  onThumbsDown?: () => void;
  onRetry?: () => void;
  /** Open the verdict detail drawer for a single verdict. */
  onShowVerdict?: (verdict: ChatVerdictRow) => void;
  /** Send a follow-up user message (used by "ask me about this claim"). */
  onAskAboutClaim?: (verdict: ChatVerdictRow) => void;
}

type WorstStrengthTone = 'success' | 'warning' | 'error' | 'info' | 'secondary';

interface VerdictSummary {
  worstStatus: ChatVerdictRow['status'];
  worstStrength: ChatVerdictRow['evidence_strength'];
  count: number;
}

const STATUS_PRIORITY: Record<ChatVerdictRow['status'], number> = {
  contradicted: 4,
  unsupported: 3,
  unverifiable: 2,
  rhetorical: 1,
  supported: 0,
};

const STRENGTH_PRIORITY: Record<ChatVerdictRow['evidence_strength'], number> = {
  none: 4,
  weak: 3,
  mixed: 2,
  strong: 1,
};

const STATUS_TONE: Record<ChatVerdictRow['status'], WorstStrengthTone> = {
  contradicted: 'error',
  unsupported: 'warning',
  unverifiable: 'secondary',
  rhetorical: 'info',
  supported: 'success',
};

function summarizeVerdicts(verdicts: ChatVerdictRow[]): VerdictSummary | null {
  if (verdicts.length === 0) return null;
  let worstStatus: ChatVerdictRow['status'] = 'supported';
  let worstStrength: ChatVerdictRow['evidence_strength'] = 'strong';
  for (const v of verdicts) {
    if (STATUS_PRIORITY[v.status] > STATUS_PRIORITY[worstStatus]) {
      worstStatus = v.status;
    }
    if (STRENGTH_PRIORITY[v.evidence_strength] > STRENGTH_PRIORITY[worstStrength]) {
      worstStrength = v.evidence_strength;
    }
  }
  return { worstStatus, worstStrength, count: verdicts.length };
}

function chipClassForTone(tone: WorstStrengthTone): string {
  switch (tone) {
    case 'success':
      return 'bg-green-500/15 text-green-300 hover:bg-green-500/25';
    case 'warning':
      return 'bg-amber-500/15 text-amber-300 hover:bg-amber-500/25';
    case 'error':
      return 'bg-red-500/15 text-red-300 hover:bg-red-500/25';
    case 'info':
      return 'bg-sky-500/15 text-sky-300 hover:bg-sky-500/25';
    case 'secondary':
    default:
      return 'bg-zinc-500/15 text-on-surface hover:bg-zinc-500/25';
  }
}

const MessageItem = memo(function MessageItem({
  message,
  metadata,
  feedback,
  isError = false,
  hasInsight = false,
  activityList,
  verdicts,
  onCopy,
  onShare,
  onShareToFeed,
  onCreateInsight,
  onThumbsUp,
  onThumbsDown,
  onRetry,
  onShowVerdict,
  onAskAboutClaim,
}: MessageItemProps) {
  const isUser = message.role === 'user';
  const rawContent = stripContextPrefix(message.content);

  // When an activity list is present, strip it from the displayed content (for old baked-in messages)
  const content = activityList ? splitActivityContent(rawContent)[1] : rawContent;

  const messageVerdicts = useMemo(
    () => (verdicts ?? []).filter((v) => v.message_id === message.id),
    [verdicts, message.id],
  );
  const verdictSummary = useMemo(
    () => summarizeVerdicts(messageVerdicts),
    [messageVerdicts],
  );

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
          <img src="/dravr-icon.svg" alt="Dravr" className="w-8 h-8 rounded-xl" />
        )}
      </div>
      {/* Message Content */}
      <div className="flex-1 min-w-0 pt-1">
        <div className="font-medium text-on-surface text-sm mb-1">
          {isUser ? 'You' : 'Dravr'}
        </div>
        {/* Collapsible activity list (collapsed by default) */}
        {activityList && (
          <details className="mb-3">
            <summary className="cursor-pointer text-sm text-on-surface-variant hover:text-on-surface transition-colors select-none">
              Your Activities ({countActivities(activityList)})
            </summary>
            <div className="mt-2 ml-4 text-on-surface text-sm prose prose-sm prose-invert max-w-none">
              <Markdown remarkPlugins={[remarkGfm]}>{activityList}</Markdown>
            </div>
          </details>
        )}
        <div className={`text-on-surface text-sm leading-relaxed prose prose-sm prose-invert max-w-none prose-a:text-pierre-violet prose-a:underline hover:prose-a:text-pierre-violet/80 ${isError ? 'text-red-400' : ''}`}>
          <Markdown
            remarkPlugins={[remarkGfm]}
            components={{
              a: ({ href, children }) => (
                <a href={href} target="_blank" rel="noopener noreferrer" className="break-all">
                  {children}
                </a>
              ),
            }}
          >
            {linkifyUrls(content)}
          </Markdown>
        </div>
        {/* Tier 5.5 verdict chips — one summary chip + per-claim drawer triggers */}
        {!isUser && verdictSummary && messageVerdicts.length > 0 && (
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={() => onShowVerdict?.(messageVerdicts[0])}
              className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs transition-colors ${chipClassForTone(STATUS_TONE[verdictSummary.worstStatus])}`}
              title={`${verdictSummary.count} claim verdict${verdictSummary.count === 1 ? '' : 's'} — worst: ${verdictSummary.worstStatus}, ${verdictSummary.worstStrength}`}
            >
              <ShieldAlert className="w-3 h-3" />
              <span>
                {verdictSummary.count} verdict{verdictSummary.count === 1 ? '' : 's'} ·{' '}
                {verdictSummary.worstStrength}
              </span>
            </button>
            {messageVerdicts.length > 1 ? (
              <span className="text-xs text-outline">
                Click for details
              </span>
            ) : null}
            {onAskAboutClaim && messageVerdicts.length === 1 ? (
              <button
                type="button"
                onClick={() => onAskAboutClaim(messageVerdicts[0])}
                className="text-xs text-pierre-violet hover:underline"
              >
                Ask me about this claim
              </button>
            ) : null}
          </div>
        )}
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
                <span>Retry</span>
              </button>
            ) : (
              /* Normal assistant message actions */
              <>
                {/* Copy */}
                {onCopy && (
                  <button
                    onClick={onCopy}
                    className="p-0.5 text-outline hover:text-on-surface transition-colors"
                    title="Copy message"
                  >
                    <Copy className="w-3.5 h-3.5" />
                  </button>
                )}
                {/* Share - always visible */}
                {onShare && (
                  <button
                    onClick={onShare}
                    className="p-0.5 text-outline hover:text-on-surface transition-colors"
                    title="Share"
                  >
                    <Share2 className="w-3.5 h-3.5" />
                  </button>
                )}
                {/* Create Insight - only for non-insight messages */}
                {onCreateInsight && !hasInsight && (
                  <button
                    onClick={onCreateInsight}
                    className="p-0.5 text-outline hover:text-pierre-cyan transition-colors"
                    title="Create shareable insight"
                  >
                    <Lightbulb className="w-3.5 h-3.5" />
                  </button>
                )}
                {/* Share to Feed - only for insight messages */}
                {onShareToFeed && hasInsight && (
                  <button
                    onClick={onShareToFeed}
                    className="p-0.5 text-outline hover:text-on-surface transition-colors"
                    title="Share insight"
                  >
                    <Users className="w-3.5 h-3.5" />
                  </button>
                )}
                {/* Thumbs Up */}
                {onThumbsUp && (
                  <button
                    onClick={onThumbsUp}
                    className={`p-0.5 transition-colors ${
                      feedback === 'up' ? 'text-pierre-violet' : 'text-outline hover:text-on-surface'
                    }`}
                    title="Good response"
                  >
                    <ThumbsUp className={`w-3.5 h-3.5 ${feedback === 'up' ? 'fill-current' : ''}`} />
                  </button>
                )}
                {/* Thumbs Down */}
                {onThumbsDown && (
                  <button
                    onClick={onThumbsDown}
                    className={`p-0.5 transition-colors ${
                      feedback === 'down' ? 'text-red-500' : 'text-outline hover:text-on-surface'
                    }`}
                    title="Poor response"
                  >
                    <ThumbsDown className={`w-3.5 h-3.5 ${feedback === 'down' ? 'fill-current' : ''}`} />
                  </button>
                )}
                {/* Retry */}
                {onRetry && (
                  <button
                    onClick={onRetry}
                    className="p-0.5 text-outline hover:text-on-surface transition-colors"
                    title="Regenerate response"
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
      </div>
    </div>
  );
});

export default MessageItem;
