// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One conversation's transcript and composer — the part of a thread every chat surface on the web shares
// ABOUTME: The Groups tab, Home and an activity's view each own the thread's state (useChatThread) and draw it here

import { useCallback, useMemo, useState, type ReactNode, type UIEvent } from 'react';
import { trustedActionUrl, verdictSupportReference } from '@pierre/chat-utils';
import type { ChatMessageAction, ClaimVerdict } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { useInfoToast, useSuccessToast, useErrorToast } from '../ui';
import MessageList from './MessageList';
import MessageInput from './MessageInput';
import UsageWarningBanner from './UsageWarningBanner';
import VerdictDrawer, { type VerdictSource } from './VerdictDrawer';
import type { ChatThreadState } from './useChatThread';

export interface ChatThreadProps {
  /** The thread's state, owned by the host so its own chrome reads and drives the same thread. */
  thread: ChatThreadState;
  /**
   * What the conversation is called, as the host's header names it. The
   * verdict drawer names the thread a claim came from by it.
   */
  conversationTitle: string;
  /** Drawn under the usage banner, above the transcript — the host's connect nudge. */
  banner?: ReactNode;
  /**
   * Whether the transcript scrolls to its latest message when a thread
   * opens. An embedded thread leaves the page where the athlete put it.
   */
  followOnOpen: boolean;
  /**
   * Whether the composer takes focus on its own. Only on a wide screen: on a
   * phone a focus without a tap opens no keyboard, yet it would fold the tab
   * bar and Home's Today line away as if the athlete were typing.
   */
  focusComposerOnOpen: boolean;
  /** Every scroll of the transcript, for a host that follows the athlete's reading. */
  onTranscriptScroll?: (event: UIEvent<HTMLDivElement>) => void;
}

/**
 * A thread: the usage banner, the transcript, the composer and the verdict
 * drawer, with every handler a reply's controls need.
 *
 * A reply can carry links, postback buttons and claim verdicts; each is
 * handled here once, so a thread opened from an activity's view behaves
 * exactly as one opened from Home or the Groups tab.
 */
export default function ChatThread({
  thread,
  conversationTitle,
  banner,
  followOnOpen,
  focusComposerOnOpen,
  onTranscriptScroll,
}: ChatThreadProps) {
  const { t } = useTranslation();
  const showSuccessToast = useSuccessToast();
  const showInfoToast = useInfoToast();
  const showErrorToast = useErrorToast();
  const { verdicts, refetchVerdicts, seedDraft, sendTurn, threadMessages } = thread;

  // The message whose verdicts the drawer shows. The rows are written right
  // after the reply row, so a chip that landed before the read did opens the
  // drawer on a refetch rather than on nothing — joining the read a completed
  // turn already started instead of cancelling it.
  const [verdictMessageId, setVerdictMessageId] = useState<string | null>(null);
  const drawerVerdicts = useMemo(
    () => (verdictMessageId ? verdicts.filter((v) => v.message_id === verdictMessageId) : []),
    [verdicts, verdictMessageId],
  );
  const handleShowVerdict = useCallback(
    (messageVerdicts: ClaimVerdict[], messageId: string) => {
      setVerdictMessageId(messageId);
      if (messageVerdicts.length === 0) refetchVerdicts();
    },
    [refetchVerdicts],
  );

  const handleAskAboutClaim = useCallback((verdict: ClaimVerdict) => {
    seedDraft(t('app.backUpClaim', { claim: verdict.claim_text }));
  }, [seedDraft, t]);

  // The reply the drawer's verdicts were drawn from, for the pill that names
  // its conversation. The drawer opens from a chip under that reply, so the
  // reply is in the thread on screen.
  const verdictSource = useMemo<VerdictSource | undefined>(() => {
    const reply = verdictMessageId ? threadMessages.find((m) => m.id === verdictMessageId) : undefined;
    return reply ? { title: conversationTitle, content: reply.content, createdAt: reply.created_at } : undefined;
  }, [verdictMessageId, threadMessages, conversationTitle]);

  // What support needs to find a verdict. The athlete's card does not print
  // these ids; it hands them over from its actions menu.
  const handleCopyVerdictReference = useCallback((verdict: ClaimVerdict) => {
    navigator.clipboard.writeText(verdictSupportReference(verdict)).then(
      () => showSuccessToast(t('app.copiedTitle'), undefined, 2000),
      () => showErrorToast(t('app.copyFailed')),
    );
  }, [showSuccessToast, showErrorToast, t]);

  const handleCopyMessage = useCallback((content: string) => {
    navigator.clipboard.writeText(content);
    showSuccessToast(t('app.copiedTitle'), t('app.copiedBody'), 2000);
  }, [showSuccessToast, t]);

  /**
   * Press handler for a control the reply's `actions` block carried.
   *
   * A `postback` sends its `value` as the next turn, so the press flows
   * through the exact same pipeline a typed command would. A command's value
   * is the line the athlete would type, so it is what their bubble shows; any
   * other value is a suggestion's postback, which the server resolves into
   * its words, so the bubble shows the label until the turn echoes them
   * (carnet#828). A `url` opens its
   * `value` — but only after {@link trustedActionUrl} vouches for the host:
   * the value reaches the client inside a model-adjacent reply, so an
   * unvouched address is an open redirect wearing a button. A refused URL
   * opens nothing.
   */
  const handleActionClick = useCallback((action: ChatMessageAction) => {
    if (action.action_type === 'url') {
      const target = trustedActionUrl(action.value, [window.location.origin]);
      if (target) window.open(target, '_blank', 'noopener,noreferrer');
      return;
    }
    void sendTurn(action.value, action.value.startsWith('/') ? {} : { display: action.label });
  }, [sendTurn]);

  const handleShareMessage = useCallback((content: string) => {
    // Use native Web Share API if available, otherwise copy to clipboard
    if (navigator.share) {
      navigator.share({
        title: t('chat.aiInsightLabel'),
        text: content,
      }).catch(() => {
        // User cancelled share, ignore
      });
    } else {
      navigator.clipboard.writeText(content);
      showInfoToast(t('app.copiedTitle'), t('app.messageCopiedForSharing'), 2000);
    }
  }, [showInfoToast, t]);

  return (
    <>
      <UsageWarningBanner level={thread.usageStatus.level} text={thread.usageStatus.text} />
      {banner}
      {/* The scroller is the containing block for everything positioned inside
          the transcript — a screen-reader-only label is `position: absolute`,
          and without `relative` here it resolves against the tab root, where
          twenty of them stretch the outer pane to the transcript's full height
          and the first focus scrolls the header and the list off screen. */}
      <div className="relative min-h-0 flex-1 overflow-y-auto" onScroll={onTranscriptScroll}>
        <div className="px-4 py-4 md:px-6">
          {thread.roomFailed ? (
            <p className="mx-auto mb-2 max-w-[720px] text-center text-sm text-outline" data-testid="room-load-failed">
              {t('groups.roomLoadFailed')}
            </p>
          ) : null}
          <MessageList
            conversationId={thread.conversationId}
            messages={threadMessages}
            messageMetadata={thread.messageMetadata}
            messageFeedback={thread.messageFeedback}
            messageFeedbackComment={thread.messageFeedbackComment}
            messageBlocks={thread.messageBlocks}
            verdicts={verdicts}
            assistantLabel={thread.activeCoachTitle ?? undefined}
            isLoading={thread.messagesLoading}
            isStreaming={thread.isStreaming}
            streamingContent={thread.streamingContent}
            liveTrail={thread.liveTrail}
            awaitingSince={thread.awaitingSince}
            turnTrails={thread.turnTrails}
            errorMessage={thread.shownError}
            oauthNotification={thread.oauthNotification}
            onDismissError={thread.dismissError}
            onRetryError={thread.retryFailedTurn}
            onDismissOAuthNotification={thread.dismissOAuthNotification}
            onCopyMessage={handleCopyMessage}
            onShareMessage={handleShareMessage}
            onThumbsUp={thread.thumbsUp}
            onThumbsDown={thread.thumbsDown}
            onSubmitFeedbackReason={thread.submitFeedbackReason}
            onRetryMessage={thread.retryMessage}
            onShowVerdict={handleShowVerdict}
            onAskAboutClaim={handleAskAboutClaim}
            onActionClick={handleActionClick}
            followOnOpen={followOnOpen}
            hasEarlier={thread.history.hasEarlier}
            loadingEarlier={thread.history.earlierLoading}
            onReachStart={thread.reachStart}
          />
        </div>
      </div>

      <MessageInput
        value={thread.composerText}
        onChange={thread.setComposerText}
        onSend={thread.sendComposer}
        isStreaming={thread.turnInFlight}
        onStop={() => void thread.stopTurn()}
        isStopping={thread.isStopping}
        disabled={thread.usageStatus.sendDisabled}
        conversationId={thread.conversationId}
        focusOnMount={focusComposerOnOpen}
      />

      {verdictMessageId ? (
        <VerdictDrawer
          verdicts={drawerVerdicts}
          loading={thread.verdictsFetching && drawerVerdicts.length === 0}
          onClose={() => setVerdictMessageId(null)}
          onAskAboutClaim={(verdict) => {
            handleAskAboutClaim(verdict);
            setVerdictMessageId(null);
          }}
          onCopyReference={handleCopyVerdictReference}
          source={verdictSource}
        />
      ) : null}
    </>
  );
}
