// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One conversation's transcript and composer — the part of a thread every chat surface on the phone shares
// ABOUTME: The chat screen and an activity's view both mount it; each decides which conversation it is and how a turn opens one

import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Alert, Keyboard, Text, TextInput, TouchableOpacity } from 'react-native';
import * as Linking from 'expo-linking';
import { useTranslation } from '@pierre/i18n';
import { trustedActionUrl } from '@pierre/chat-utils';
import { noticeRequired } from '@pierre/shared-constants';
import type { ChatMessageAction, ClaimVerdict } from '@pierre/shared-types';

import { Sheet } from '../../components/ui';
import { OAuthCredentialsSection } from '../../components/OAuthCredentialsSection';
import { ProviderNoticeSheet } from '../../components/ProviderNotice';
import { ChatInputBar } from './ChatInputBar';
import { ChatProgressStrip } from './ChatProgressStrip';
import { MessageList } from './MessageList';
import { UsageWarningBanner } from './UsageWarningBanner';
import { VerdictSheet } from './VerdictSheet';
import { useChatVoiceInput } from './useChatVoiceInput';
import type { useMessages } from './useMessages';
import type { useProviderStatus } from './useProviderStatus';
import type { useUsageStatus } from './useUsageStatus';

export interface ChatThreadProps {
  /** The conversation on screen, or `null` before its first turn. */
  conversationId: string | null;
  /** The transcript's state, owned by the host so it can read and reload it. */
  messagesHook: ReturnType<typeof useMessages>;
  usageStatus: ReturnType<typeof useUsageStatus>;
  providerStatus: ReturnType<typeof useProviderStatus>;
  /** The host is still resolving which conversation this is. */
  isLoading: boolean;
  /** The composer's text, held by the host so a navigation can draft into it. */
  inputText: string;
  onChangeInputText: (text: string) => void;
  inputRef: React.RefObject<TextInput | null>;
  /**
   * Send one line as the next turn. The host resolves the conversation —
   * opening one when there is none — so every turn, typed or pressed, goes
   * through its one implementation.
   */
  sendText: (text: string) => Promise<void>;
  /** Drawn above the transcript, scrolling with it. */
  header?: React.ReactElement;
  /** How the transcript follows its content; the list's own scroll-to-end when absent. */
  onScrollToBottom?: () => void;
}

/**
 * A thread: the transcript, the progress line, the usage banner and the
 * composer, with every handler a reply's controls need.
 *
 * A reply can carry links, postback buttons, a reconnect prompt and claim
 * verdicts; each is handled here once, so a thread opened from an activity's
 * view behaves exactly as one opened from the chat tab.
 */
export function ChatThread({
  conversationId,
  messagesHook,
  usageStatus,
  providerStatus,
  isLoading,
  inputText,
  onChangeInputText,
  inputRef,
  sendText,
  header,
  onScrollToBottom,
}: ChatThreadProps) {
  const { t } = useTranslation();
  // The message whose verdicts the sheet shows, or `null` while it is closed.
  const [verdictMessageId, setVerdictMessageId] = useState<string | null>(null);
  // The provider whose notice is on screen before a reconnect starts its
  // OAuth flow (WHOOP, until the account accepts its owner authorization).
  const [noticeFor, setNoticeFor] = useState<string | null>(null);

  // Opening the keyboard shortens the visible list. `onContentSizeChange` only
  // fires when the CONTENT changes, so tapping into the composer on an existing
  // thread left the newest messages above the fold with nothing to bring them
  // back.
  const scrollToBottom = onScrollToBottom ?? messagesHook.scrollToBottom;
  useEffect(() => {
    const shown = Keyboard.addListener('keyboardDidShow', scrollToBottom);
    return () => shown.remove();
  }, [scrollToBottom]);

  // Voice input with chat-specific error handling
  const voiceInput = useChatVoiceInput(
    (text) => onChangeInputText(text),
    onChangeInputText
  );

  // A turn's pre-turn quota check reports its counters as a `notice` reply
  // block. Hand it to the banner, which is the one place a cap is stated.
  const { quotaNotice } = messagesHook;
  const { applyNotice } = usageStatus;
  useEffect(() => {
    if (quotaNotice) applyNotice(quotaNotice);
  }, [quotaNotice, applyNotice]);

  const handleOpenUrl = useCallback(async (url: string) => {
    try {
      let parsedUrl: URL;
      try {
        parsedUrl = new URL(url);
      } catch {
        console.error('Invalid URL:', url);
        Alert.alert(t('app.linkErrorTitle'), t('app.linkInvalidFormat'));
        return;
      }

      const scheme = parsedUrl.protocol.toLowerCase();
      if (scheme !== 'http:' && scheme !== 'https:') {
        console.warn('Blocked non-HTTP URL scheme:', scheme);
        Alert.alert(t('app.linkBlockedTitle'), t('app.linkBlockedBody'));
        return;
      }

      await Linking.openURL(url);
    } catch (error) {
      console.error('Failed to open URL:', error);
      Alert.alert(t('app.linkErrorTitle'), t('app.linkOpenFailed'));
    }
  }, [t]);

  const handleSendMessage = useCallback(async () => {
    const messageText = inputText.trim();
    if (!messageText) return;
    onChangeInputText('');
    await sendText(messageText);
  }, [inputText, onChangeInputText, sendText]);

  /**
   * Press handler for a control the reply's `actions` block carried.
   *
   * A `postback` sends its `value` as the next turn, so the press flows
   * through the same dispatch pipeline a typed command would. A `url` opens
   * its `value` in the system browser — but only after `trustedActionUrl`
   * vouches for the host: the value reaches the client inside a
   * model-adjacent reply, so an unvouched address is an open redirect wearing
   * a button. A refused URL opens nothing.
   */
  const handleActionClick = useCallback(
    async (action: ChatMessageAction) => {
      if (action.action_type === 'url') {
        const target = trustedActionUrl(action.value, [
          process.env.EXPO_PUBLIC_API_URL ?? '',
        ]);
        if (target) await handleOpenUrl(target);
        return;
      }
      await sendText(action.value);
    },
    [handleOpenUrl, sendText],
  );

  // Retry, and the feedback handlers, name the open conversation so the hook
  // can re-send or persist thumbs up/down and an optional reason against it.
  const handleRetryMessage = useCallback(async (messageId: string) => {
    if (!conversationId) return;
    await messagesHook.retryMessage(messageId, conversationId);
  }, [messagesHook, conversationId]);

  const handleThumbsUp = useCallback((messageId: string) => {
    if (!conversationId) return;
    void messagesHook.handleThumbsUp(messageId, conversationId);
  }, [messagesHook, conversationId]);

  const handleThumbsDown = useCallback((messageId: string) => {
    if (!conversationId) return;
    void messagesHook.handleThumbsDown(messageId, conversationId);
  }, [messagesHook, conversationId]);

  const handleSubmitFeedbackReason = useCallback((messageId: string, comment: string) => {
    if (!conversationId) return;
    void messagesHook.submitFeedbackReason(messageId, conversationId, comment);
  }, [messagesHook, conversationId]);

  // The rows are written right after the reply row, so a chip that landed
  // before the read did opens the sheet on a re-read rather than on nothing.
  const { refreshVerdicts, verdicts, verdictsLoading } = messagesHook;
  const sheetVerdicts = useMemo(
    () => (verdictMessageId ? verdicts.filter((v) => v.message_id === verdictMessageId) : []),
    [verdicts, verdictMessageId],
  );
  const handleShowVerdict = useCallback((rows: ClaimVerdict[], messageId: string) => {
    setVerdictMessageId(messageId);
    if (rows.length === 0 && conversationId) void refreshVerdicts(conversationId);
  }, [refreshVerdicts, conversationId]);

  const handleAskAboutClaim = useCallback((verdict: ClaimVerdict) => {
    onChangeInputText(t('app.backUpClaim', { claim: verdict.claim_text }));
    setVerdictMessageId(null);
  }, [onChangeInputText, t]);

  /**
   * Authorize a provider from a reply that asks for it.
   *
   * `WebBrowser.openAuthSessionAsync` presents a sheet over the app that
   * hands the callback back to it. Opening the reply's URL with the generic
   * opener instead sends the athlete to Safari, where the callback has
   * nowhere to return to.
   */
  const handleConnectProvider = useCallback(async (provider: string) => {
    // A provider whose notice the account has not accepted (WHOOP's owner
    // authorization) states it first; its Continue starts the flow.
    const status = providerStatus.connectedProviders.find((p) => p.provider === provider);
    if (noticeRequired(provider, status?.consent_required)) {
      setNoticeFor(provider);
      return;
    }
    await providerStatus.handleConnectProvider(provider);
  }, [providerStatus]);

  return (
    <>
      <VerdictSheet
        visible={verdictMessageId !== null}
        verdicts={sheetVerdicts}
        loading={verdictsLoading && sheetVerdicts.length === 0}
        onClose={() => setVerdictMessageId(null)}
        onAskAboutClaim={handleAskAboutClaim}
      />

      <MessageList
        messages={messagesHook.messages}
        isLoading={isLoading}
        isSending={messagesHook.isSending}
        messageFeedback={messagesHook.messageFeedback}
        messageFeedbackComment={messagesHook.messageFeedbackComment}
        messageBlocks={messagesHook.messageBlocks}
        verdicts={messagesHook.verdicts}
        flatListRef={messagesHook.flatListRef}
        onScrollToBottom={scrollToBottom}
        onThumbsUp={handleThumbsUp}
        onThumbsDown={handleThumbsDown}
        onSubmitFeedbackReason={handleSubmitFeedbackReason}
        onRetryMessage={handleRetryMessage}
        onOpenUrl={handleOpenUrl}
        onReconnectProvider={handleConnectProvider}
        onActionClick={handleActionClick}
        onShowVerdict={handleShowVerdict}
        header={header}
      />

      <ChatProgressStrip statusText={messagesHook.progressText} />

      <UsageWarningBanner level={usageStatus.level} message={usageStatus.message} />

      <ChatInputBar
        inputText={inputText}
        partialTranscript={voiceInput.partialTranscript}
        isListening={voiceInput.isListening}
        isSending={messagesHook.isSending}
        disabled={usageStatus.sendDisabled}
        voiceAvailable={voiceInput.isAvailable}
        inputRef={inputRef}
        onChangeText={onChangeInputText}
        onVoicePress={voiceInput.handleVoicePress}
        onSendMessage={handleSendMessage}
      />

      {/* A reply asked for provider credentials the app does not hold yet. */}
      <ProviderNoticeSheet
        provider={noticeFor}
        onCancel={() => setNoticeFor(null)}
        onAccept={() => {
          const accepted = noticeFor;
          setNoticeFor(null);
          if (accepted) void providerStatus.handleConnectProvider(accepted, undefined, true);
        }}
      />

      <Sheet
        visible={providerStatus.needsCredentialsProvider !== null}
        onClose={() => providerStatus.setNeedsCredentialsProvider(null)}
        testID="oauth-credentials-sheet"
        flush
      >
        <OAuthCredentialsSection />
        <TouchableOpacity
          className="mt-4 py-3 items-center"
          onPress={() => providerStatus.setNeedsCredentialsProvider(null)}
        >
          <Text className="text-base text-text-tertiary">{t('common.close')}</Text>
        </TouchableOpacity>
      </Sheet>
    </>
  );
}
