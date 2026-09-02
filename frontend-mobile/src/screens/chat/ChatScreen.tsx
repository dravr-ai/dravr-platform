// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Main chat screen orchestrator importing decomposed hooks and components
// ABOUTME: Coordinates conversation, message, provider and voice state, and the thread's info sheet

import React, { useState, useRef, useEffect, useCallback, useMemo } from 'react';
import { View, Text, TextInput, TouchableOpacity, Modal, Alert } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import * as Linking from 'expo-linking';
import { useRouter, useLocalSearchParams, useFocusEffect } from 'expo-router';

import { useAuth } from '../../contexts/AuthContext';
import { PromptDialog } from '../../components/ui';
import { trackMobile } from '../../services/analytics';
import { trustedActionUrl } from '@pierre/chat-utils';
import type { ChatMessageAction, ClaimVerdict } from '@pierre/shared-types';

import { ChatHeader } from './ChatHeader';
import { ChatPlusFlows } from './ChatPlusFlows';
import { useChatPlusActions } from './useChatPlusActions';
import { CHAT_LIST_ROUTE, NEW_CONVERSATION_ID, threadHref } from '../../navigation/routes';
import { ChatInputBar } from './ChatInputBar';
import { tabBarBottomOffset } from '../../components/ui';
import { useKeyboardOffset } from '../../hooks/useKeyboardOffset';
import { ChatProgressStrip } from './ChatProgressStrip';
import { ConversationInfoSheet } from './ConversationInfoSheet';
import { MessageList } from './MessageList';
import { ProviderModal } from './ProviderModal';
import { SciotteLoginModal } from '../../components/SciotteLoginModal';
import { IntervalsIcuLinkModal } from '../../components/IntervalsIcuLinkModal';
import { OAuthCredentialsSection } from '../../components/OAuthCredentialsSection';
import { useConversations } from './useConversations';
import { useMarkConversationRead } from './useMarkConversationRead';
import { useMessages } from './useMessages';
import { useProviderStatus } from './useProviderStatus';
import { useChatVoiceInput } from './useChatVoiceInput';
import { useUsageStatus } from './useUsageStatus';
import { UsageWarningBanner } from './UsageWarningBanner';
import { VerdictSheet } from './VerdictSheet';
import { useTranslation } from '@pierre/i18n';

export function ChatScreen() {
  const { t } = useTranslation();
  const { isAuthenticated } = useAuth();
  const insets = useSafeAreaInsets();
  // One keyboard reading, shared by the composer and the list. They used to
  // disagree: the composer listened and moved, the list reserved a fixed 140dp
  // and did not, so the newest messages hid behind the raised composer.
  const keyboard = useKeyboardOffset();
  // The resting position, from the device's REAL bottom inset rather than the
  // hardcoded 40dp that assumed every phone has a home indicator.
  const composerResting = tabBarBottomOffset(insets.bottom);
  const router = useRouter();
  const params = useLocalSearchParams<{ conversationId?: string; draft?: string; send?: string }>();
  const inputRef = useRef<TextInput>(null);

  // UI State
  const [inputText, setInputText] = useState('');
  const [infoVisible, setInfoVisible] = useState(false);
  const [renamePromptVisible, setRenamePromptVisible] = useState(false);
  const [renameConversationId, setRenameConversationId] = useState<string | null>(null);
  const [renameDefaultTitle, setRenameDefaultTitle] = useState('');
  const [sciotteTarget, setSciotteTarget] = useState<'strava' | 'garmin' | null>(null);
  const [intervalsModalVisible, setIntervalsModalVisible] = useState(false);
  // The message whose verdicts the sheet shows, or `null` while it is closed.
  const [verdictMessageId, setVerdictMessageId] = useState<string | null>(null);

  // Custom hooks
  const conversations = useConversations();
  const messagesHook = useMessages();

  // Opening the keyboard shortens the visible list. `onContentSizeChange` only
  // fires when the CONTENT changes, so tapping into the composer on an existing
  // thread left the newest messages above the fold with nothing to bring them
  // back.
  const scrollToBottom = messagesHook.scrollToBottom;
  useEffect(() => {
    if (keyboard.height > 0) {
      scrollToBottom();
    }
  }, [keyboard.height, scrollToBottom]);
  const providerStatus = useProviderStatus();
  const usageStatus = useUsageStatus();
  // The flow state behind the info sheet's "Participants" row. The tab bar's
  // "+" holds its own copy for the same thread, so "add someone to this
  // discussion" and "Participants" open the same control either way.
  const chatPlus = useChatPlusActions(conversations.currentConversation?.id ?? null);

  const { messages } = messagesHook;
  const lastMessageId = messages.length > 0 ? messages[messages.length - 1].id : null;
  // Reading is looking: the marker advances only while this screen is focused
  // and the app is awake, and again on every new last message.
  useMarkConversationRead({
    conversationId: conversations.currentConversation?.id ?? null,
    lastMessageId,
  });

  // The thread is pushed over the conversation list; a deep link or a cold
  // start can land here with nothing beneath, so fall back to the list.
  const goBackToList = useCallback(() => {
    if (router.canGoBack()) {
      router.back();
    } else {
      router.replace(CHAT_LIST_ROUTE);
    }
  }, [router]);

  // Voice input with chat-specific error handling
  const voiceInput = useChatVoiceInput(
    (text) => setInputText(text),
    setInputText
  );

  // Load data when authenticated
  useEffect(() => {
    if (isAuthenticated) {
      conversations.loadConversations();
      providerStatus.loadProviderStatus();
    }
    // These functions are stable from hooks, intentionally omit to avoid loops
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isAuthenticated]);

  // Refresh provider status on focus, and re-read the open thread
  useFocusEffect(
    useCallback(() => {
      if (isAuthenticated) {
        providerStatus.loadProviderStatus();
        // Conversations can be opened from outside this screen — an invite
        // deep link and the "+" both route here by id. The id resolves against
        // this list, so a stale list lands the athlete on an empty composer
        // instead of the conversation they just opened.
        void conversations.loadConversations();
        // Messaging turns arrive async via inbound webhook with no push to the
        // app. Reload the open conversation on focus so a reply sent from
        // Telegram (or any channel) appears without a manual pull-to-refresh.
        // Skipped mid-send so an in-flight optimistic turn isn't clobbered.
        const openId = conversations.currentConversation?.id;
        if (openId && !messagesHook.isSending) {
          void messagesHook.loadMessages(openId);
        }
      }
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [isAuthenticated, conversations.currentConversation?.id, messagesHook.isSending])
  );

  // Load messages when conversation changes
  useEffect(() => {
    if (conversations.currentConversation) {
      if (conversations.justCreatedConversationRef.current === conversations.currentConversation.id) {
        conversations.justCreatedConversationRef.current = null;
        return;
      }
      messagesHook.loadMessages(conversations.currentConversation.id);
    } else {
      messagesHook.clearMessages();
    }
    // Intentionally only depend on currentConversation to avoid infinite loops
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conversations.currentConversation]);

  // Handle navigation params for conversation selection
  // Clear conversation when navigating to chat without a conversationId (or with 'new')
  useEffect(() => {
    const conversationId = params?.conversationId;
    if (
      (conversationId === undefined || conversationId === NEW_CONVERSATION_ID) &&
      conversations.currentConversation !== null
    ) {
      conversations.setCurrentConversation(null);
      messagesHook.clearMessages();
    }
    // Only depend on conversationId value, not the params object reference
    // (useLocalSearchParams returns a new object each render unlike route.params)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params.conversationId]);

  useEffect(() => {
    const conversationId = params?.conversationId;
    if (conversationId && conversations.conversations.length > 0) {
      const conversation = conversations.conversations.find(c => c.id === conversationId);
      const shouldUpdate = conversation && (
        conversation.id !== conversations.currentConversation?.id ||
        (!conversations.currentConversation?.title && conversation.title)
      );
      if (shouldUpdate) {
        conversations.setCurrentConversation(conversation);
      }
    }
    // currentConversation intentionally omitted - including it would cause infinite loops
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params?.conversationId, conversations.conversations]);

  // URL handling
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

  // A turn's pre-turn quota check reports its counters as a `notice` reply
  // block. Hand it to the banner, which is the one place a cap is stated.
  const { quotaNotice } = messagesHook;
  const { applyNotice } = usageStatus;
  useEffect(() => {
    if (quotaNotice) applyNotice(quotaNotice);
  }, [quotaNotice, applyNotice]);

  /**
   * Send one line as the next turn, creating the thread when there is none.
   *
   * Everything that produces a turn goes through here — the composer, a
   * reply's postback button, a command an info sheet issues, and the `send`
   * param an invite link or a t('app.newGroupChat') prompt arrives with — so quota
   * accounting and thread creation have exactly one implementation.
   */
  const sendText = useCallback(async (text: string) => {
    const trimmed = text.trim();
    if (!trimmed || messagesHook.isSending) return;

    let conversationId = conversations.currentConversation?.id;
    if (!conversationId) {
      const newConversation = await conversations.createConversation({
        title: trimmed.slice(0, 50),
      });
      if (!newConversation) return;
      conversationId = newConversation.id;
    }

    try {
      trackMobile({ name: 'feature_engaged', props: { feature: 'chat_message_sent' } });
      const rotatedTo = await messagesHook.sendTurn(conversationId, trimmed);
      // `/reset` archives this thread and continues on a fresh one. Resolve the
      // new row before navigating, so the screen lands on a thread it can
      // actually draw; `replace`, not `push`, because Back must not return the
      // athlete to the thread they just abandoned.
      if (rotatedTo && rotatedTo !== conversationId) {
        const opened = await conversations.switchToConversation(rotatedTo);
        if (opened) router.replace(threadHref(rotatedTo));
      }
    } finally {
      usageStatus.invalidate();
    }
  }, [messagesHook, conversations, usageStatus, router]);

  const handleSendMessage = useCallback(async () => {
    const messageText = inputText.trim();
    if (!messageText) return;
    setInputText('');
    await sendText(messageText);
  }, [inputText, sendText]);

  // A navigation may arrive with composer intent: `draft` fills the composer
  // and waits for the athlete, `send` runs once. Both are command text built
  // by COMMAND_DRAFTS, and each is honoured once per value so a re-render on
  // the same route never re-sends it.
  const draftedRef = useRef<string | null>(null);
  useEffect(() => {
    const draft = typeof params.draft === 'string' ? params.draft : null;
    if (!draft || draftedRef.current === draft) return;
    draftedRef.current = draft;
    setInputText(draft);
  }, [params.draft]);

  const sentRef = useRef<string | null>(null);
  useEffect(() => {
    const send = typeof params.send === 'string' ? params.send : null;
    if (!send || sentRef.current === send) return;
    sentRef.current = send;
    void sendText(send);
  }, [params.send, sendText]);

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

  // Retry message
  const handleRetryMessage = useCallback(async (messageId: string) => {
    if (!conversations.currentConversation?.id) return;
    await messagesHook.retryMessage(messageId, conversations.currentConversation.id);
  }, [messagesHook, conversations.currentConversation?.id]);

  // Feedback handlers inject the active conversation id (mirrors retry) so the
  // hook can persist thumbs up/down + an optional reason against the server.
  const handleThumbsUp = useCallback((messageId: string) => {
    if (!conversations.currentConversation?.id) return;
    void messagesHook.handleThumbsUp(messageId, conversations.currentConversation.id);
  }, [messagesHook, conversations.currentConversation?.id]);

  const handleThumbsDown = useCallback((messageId: string) => {
    if (!conversations.currentConversation?.id) return;
    void messagesHook.handleThumbsDown(messageId, conversations.currentConversation.id);
  }, [messagesHook, conversations.currentConversation?.id]);

  const handleSubmitFeedbackReason = useCallback((messageId: string, comment: string) => {
    if (!conversations.currentConversation?.id) return;
    void messagesHook.submitFeedbackReason(messageId, conversations.currentConversation.id, comment);
  }, [messagesHook, conversations.currentConversation?.id]);

  // The rows are written right after the reply row, so a chip that landed
  // before the read did opens the sheet on a re-read rather than on nothing.
  const { refreshVerdicts, verdicts, verdictsLoading } = messagesHook;
  const sheetVerdicts = useMemo(
    () => (verdictMessageId ? verdicts.filter((v) => v.message_id === verdictMessageId) : []),
    [verdicts, verdictMessageId],
  );
  const handleShowVerdict = useCallback((rows: ClaimVerdict[], messageId: string) => {
    setVerdictMessageId(messageId);
    const conversationId = conversations.currentConversation?.id;
    if (rows.length === 0 && conversationId) void refreshVerdicts(conversationId);
  }, [refreshVerdicts, conversations.currentConversation?.id]);

  const handleAskAboutClaim = useCallback((verdict: ClaimVerdict) => {
    setInputText(t('app.backUpClaim', { claim: verdict.claim_text }));
    setVerdictMessageId(null);
  }, [t]);

  /**
   * Authorize a provider, from the picker or from a reply that asks for it.
   *
   * One path for both: `WebBrowser.openAuthSessionAsync`, a sheet presented
   * over the app that hands the callback back to it. Opening the reply's URL
   * with the generic opener instead sends the athlete to Safari, where the
   * callback has nowhere to return to.
   */
  const handleConnectProvider = useCallback(async (provider: string) => {
    await providerStatus.handleConnectProvider(provider);
  }, [providerStatus]);

  const handleProviderSelect = useCallback((provider: string) => {
    providerStatus.setSelectedProvider(provider);
    providerStatus.setProviderModalVisible(false);
  }, [providerStatus]);

  const handleProviderModalClose = useCallback(() => {
    providerStatus.setProviderModalVisible(false);
  }, [providerStatus]);

  // Info sheet handlers
  const openInfoSheet = useCallback(() => {
    if (!conversations.currentConversation) return;
    setInfoVisible(true);
  }, [conversations.currentConversation]);

  const handleInfoRename = useCallback(() => {
    setInfoVisible(false);
    if (conversations.currentConversation) {
      const title = conversations.currentConversation.title || t('app.chatUntitled');
      setRenameConversationId(conversations.currentConversation.id);
      setRenameDefaultTitle(title);
      setRenamePromptVisible(true);
    }
  }, [conversations.currentConversation, t]);

  const handleInfoParticipants = useCallback(() => {
    setInfoVisible(false);
    if (conversations.currentConversation) {
      chatPlus.flows.openParticipants();
    }
  }, [conversations.currentConversation, chatPlus.flows]);

  const handleInfoDelete = useCallback(() => {
    setInfoVisible(false);
    if (!conversations.currentConversation) return;

    Alert.alert(
      t('app.convDeleteTitle'),
      t('app.confirmDeleteConversation', { title: conversations.currentConversation.title || t('app.thisConversation') }),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('common.delete'),
          style: 'destructive',
          onPress: async () => {
            await conversations.deleteConversation(conversations.currentConversation!.id);
            // The thread is gone; the list is where the athlete goes next.
            goBackToList();
          },
        },
      ]
    );
  }, [conversations, goBackToList, t]);

  const handleRenameSubmit = useCallback(async (newTitle: string) => {
    setRenamePromptVisible(false);
    if (!renameConversationId) return;
    await conversations.renameConversation(renameConversationId, newTitle);
    setRenameConversationId(null);
    setRenameDefaultTitle('');
  }, [renameConversationId, conversations]);

  const handleRenameCancel = useCallback(() => {
    setRenamePromptVisible(false);
    setRenameConversationId(null);
    setRenameDefaultTitle('');
  }, []);

  return (
    <View className="flex-1 bg-background-primary" testID="chat-screen">
      <View
        className="flex-1"
      >
        <ChatHeader
          currentConversation={conversations.currentConversation}
          insetTop={insets.top}
          onBackPress={goBackToList}
          onTitlePress={openInfoSheet}
        />

        <ChatPlusFlows flows={chatPlus.flows} />

        <ConversationInfoSheet
          visible={infoVisible}
          conversation={conversations.currentConversation}
          onClose={() => setInfoVisible(false)}
          onSendCommand={(command) => void sendText(command)}
          onRename={handleInfoRename}
          onParticipants={handleInfoParticipants}
          onDelete={handleInfoDelete}
          onLeaveThread={goBackToList}
        />

        <VerdictSheet
          visible={verdictMessageId !== null}
          verdicts={sheetVerdicts}
          loading={verdictsLoading && sheetVerdicts.length === 0}
          onClose={() => setVerdictMessageId(null)}
          onAskAboutClaim={handleAskAboutClaim}
        />

        <MessageList
          messages={messagesHook.messages}
          isLoading={conversations.isLoading}
          isSending={messagesHook.isSending}
          messageFeedback={messagesHook.messageFeedback}
          messageFeedbackComment={messagesHook.messageFeedbackComment}
          messageBlocks={messagesHook.messageBlocks}
          verdicts={messagesHook.verdicts}
          flatListRef={messagesHook.flatListRef}
          onScrollToBottom={messagesHook.scrollToBottom}
          onThumbsUp={handleThumbsUp}
          onThumbsDown={handleThumbsDown}
          onSubmitFeedbackReason={handleSubmitFeedbackReason}
          onRetryMessage={handleRetryMessage}
          onOpenUrl={handleOpenUrl}
          onReconnectProvider={handleConnectProvider}
          onActionClick={handleActionClick}
          onShowVerdict={handleShowVerdict}
          bottomInset={Math.max(composerResting, keyboard.height)}
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
          onChangeText={setInputText}
          onVoicePress={voiceInput.handleVoicePress}
          onSendMessage={handleSendMessage}
          restingOffset={composerResting}
          keyboardHeight={keyboard.height}
          keyboardDuration={keyboard.duration}
        />

        <ProviderModal
          visible={providerStatus.providerModalVisible}
          providers={providerStatus.connectedProviders}
          connectingProvider={providerStatus.connectingProvider}
          onClose={handleProviderModalClose}
          onSelectConnected={handleProviderSelect}
          onConnectProvider={handleConnectProvider}
          onConnectSciotte={(target) => {
            providerStatus.setProviderModalVisible(false);
            setSciotteTarget(target);
          }}
          onConnectIntervals={() => {
            providerStatus.setProviderModalVisible(false);
            setIntervalsModalVisible(true);
          }}
        />

        <SciotteLoginModal
          visible={sciotteTarget !== null}
          onClose={() => setSciotteTarget(null)}
          onConnected={() => {
            providerStatus.loadProviderStatus();
            setSciotteTarget(null);
          }}
          target={sciotteTarget ?? 'strava'}
        />

        <IntervalsIcuLinkModal
          visible={intervalsModalVisible}
          onClose={() => setIntervalsModalVisible(false)}
          onConnected={() => {
            providerStatus.loadProviderStatus();
            setIntervalsModalVisible(false);
          }}
        />

        {providerStatus.needsCredentialsProvider !== null && (
          <Modal visible animationType="slide" transparent onRequestClose={() => providerStatus.setNeedsCredentialsProvider(null)}>
            <View className="flex-1 bg-black/60 justify-end">
              <View
                className="bg-background-primary rounded-t-3xl pt-4 pb-10 px-4"
                onStartShouldSetResponder={() => true}
              >
                <View className="items-center mb-2">
                  <View className="w-10 h-1 rounded-full bg-border-default" />
                </View>
                <OAuthCredentialsSection />
                <TouchableOpacity
                  className="mt-4 py-3 items-center"
                  onPress={() => providerStatus.setNeedsCredentialsProvider(null)}
                >
                  <Text className="text-base text-text-tertiary">{t('common.close')}</Text>
                </TouchableOpacity>
              </View>
            </View>
          </Modal>
        )}

        <PromptDialog
          visible={renamePromptVisible}
          title={t('app.chatRenameTitle')}
          message="Enter a new name for this conversation"
          defaultValue={renameDefaultTitle}
          submitText={t('common.save')}
          cancelText={t('common.cancel')}
          onSubmit={handleRenameSubmit}
          onCancel={handleRenameCancel}
          testID="rename-conversation-dialog"
        />

      </View>
    </View>
  );
}
