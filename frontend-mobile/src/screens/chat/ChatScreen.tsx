// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Main chat screen orchestrator importing decomposed hooks and components
// ABOUTME: Coordinates conversation, message, provider, coach, and voice input state

import React, { useState, useRef, useEffect, useCallback } from 'react';
import { View, Text, TextInput, TouchableOpacity, Modal, Alert } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import * as Linking from 'expo-linking';
import { useRouter, useLocalSearchParams, useFocusEffect } from 'expo-router';

import { useAuth } from '../../contexts/AuthContext';
import { PromptDialog } from '../../components/ui';
import { trackMobile } from '../../services/analytics';
import { trustedActionUrl } from '@pierre/chat-utils';
import type { ChatMessageAction } from '@pierre/shared-types';
import type { Coach } from '../../types';

import { ChatHeader } from './ChatHeader';
import { ChatPlusSheet } from './ChatPlusSheet';
import { ChatPlusFlows } from './ChatPlusFlows';
import { useChatPlusActions } from './useChatPlusActions';
import { CHAT_LIST_ROUTE } from '../../navigation/routes';
import { ChatInputBar } from './ChatInputBar';
import { ChatProgressStrip } from './ChatProgressStrip';
import { MessageList } from './MessageList';
import { ProviderModal } from './ProviderModal';
import { SciotteLoginModal } from '../../components/SciotteLoginModal';
import { IntervalsIcuLinkModal } from '../../components/IntervalsIcuLinkModal';
import { OAuthCredentialsSection } from '../../components/OAuthCredentialsSection';
import { useConversations } from './useConversations';
import { useMessages } from './useMessages';
import { useProviderStatus } from './useProviderStatus';
import { useCoachSelection } from './useCoachSelection';
import { useChatVoiceInput } from './useChatVoiceInput';
import { useUsageStatus } from './useUsageStatus';
import { UsageWarningBanner } from './UsageWarningBanner';

export function ChatScreen() {
  const { isAuthenticated } = useAuth();
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const params = useLocalSearchParams<{ conversationId?: string }>();
  const inputRef = useRef<TextInput>(null);

  // UI State
  const [inputText, setInputText] = useState('');
  const [actionMenuVisible, setActionMenuVisible] = useState(false);
  const [renamePromptVisible, setRenamePromptVisible] = useState(false);
  const [renameConversationId, setRenameConversationId] = useState<string | null>(null);
  const [renameDefaultTitle, setRenameDefaultTitle] = useState('');
  const [plusVisible, setPlusVisible] = useState(false);
  const [pendingPrompt, setPendingPrompt] = useState<string | null>(null);
  const [sciotteTarget, setSciotteTarget] = useState<'strava' | 'garmin' | null>(null);
  const [intervalsModalVisible, setIntervalsModalVisible] = useState(false);

  // Custom hooks
  const conversations = useConversations();
  const messagesHook = useMessages();
  const providerStatus = useProviderStatus();
  const coachSelection = useCoachSelection();
  const usageStatus = useUsageStatus();
  // The "+" and the title menu's "Participants" share one flow state, so
  // "add someone to this discussion" and "Participants" open the same sheet.
  const chatPlus = useChatPlusActions(conversations.currentConversation?.id ?? null);

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
      coachSelection.loadCoaches();
    }
    // These functions are stable from hooks, intentionally omit to avoid loops
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isAuthenticated]);

  // Refresh coaches and provider status on focus so newly created coaches appear
  useFocusEffect(
    useCallback(() => {
      if (isAuthenticated) {
        coachSelection.loadCoaches();
        providerStatus.loadProviderStatus();
        // Conversations can be opened from outside this screen — the group
        // detail screen creates a group-scoped one and routes here by id. The
        // id resolves against this list, so a stale list lands the athlete on
        // an empty composer instead of the conversation they just opened.
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
    if ((conversationId === undefined || conversationId === 'new') && conversations.currentConversation !== null) {
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
        Alert.alert('Error', 'Invalid link format');
        return;
      }

      const scheme = parsedUrl.protocol.toLowerCase();
      if (scheme !== 'http:' && scheme !== 'https:') {
        console.warn('Blocked non-HTTP URL scheme:', scheme);
        Alert.alert('Blocked', 'Only HTTP and HTTPS links can be opened.');
        return;
      }

      await Linking.openURL(url);
    } catch (error) {
      console.error('Failed to open URL:', error);
      Alert.alert('Error', 'Failed to open link');
    }
  }, []);

  // A turn's pre-turn quota check reports its counters as a `notice` reply
  // block. Hand it to the banner, which is the one place a cap is stated.
  const { quotaNotice } = messagesHook;
  const { applyNotice } = usageStatus;
  useEffect(() => {
    if (quotaNotice) applyNotice(quotaNotice);
  }, [quotaNotice, applyNotice]);

  // Message sending
  const handleSendMessage = useCallback(async () => {
    if (!inputText.trim() || messagesHook.isSending) return;

    const messageText = inputText.trim();
    setInputText('');

    let conversationId = conversations.currentConversation?.id;
    if (!conversationId) {
      const newConversation = await conversations.createConversation({
        title: messageText.slice(0, 50),
      });
      if (!newConversation) return;
      conversationId = newConversation.id;
    }

    try {
      trackMobile({ name: 'feature_engaged', props: { feature: 'chat_message_sent' } });
      await messagesHook.sendTurn(conversationId, messageText);
    } finally {
      usageStatus.invalidate();
    }
  }, [inputText, messagesHook, conversations, usageStatus]);

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
      // postback: send value as next turn. Uses existing handleSendMessage
      // after seeding the composer so quota + error handling stay uniform.
      setInputText(action.value);
      // Defer so React commits setInputText before handleSendMessage reads.
      setTimeout(() => {
        void handleSendMessage();
      }, 0);
    },
    [handleOpenUrl, handleSendMessage],
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

  // Coach selection handling
  const handleCoachSelect = useCallback(async (coach: Coach) => {
    await coachSelection.handleCoachSelect(coach, {
      isSending: messagesHook.isSending,
      hasConnectedProvider: providerStatus.hasConnectedProvider,
      selectedProvider: providerStatus.selectedProvider,
      connectedProviders: providerStatus.connectedProviders,
      setSelectedProvider: providerStatus.setSelectedProvider,
      setProviderModalVisible: providerStatus.setProviderModalVisible,
      startCoachConversation: async (coach) => {
        await coachSelection.startCoachConversation(coach, {
          createConversation: conversations.createConversation,
          setMessages: messagesHook.setMessages,
          setIsSending: messagesHook.setIsSending,
          scrollToBottom: messagesHook.scrollToBottom,
          setMessageBlocks: messagesHook.setMessageBlocks,
        });
      },
    });
  }, [coachSelection, messagesHook, providerStatus, conversations]);

  // Start coach conversation helper
  const startCoachConversation = useCallback(async (coach: Coach) => {
    await coachSelection.startCoachConversation(coach, {
      createConversation: conversations.createConversation,
      setMessages: messagesHook.setMessages,
      setIsSending: messagesHook.setIsSending,
      scrollToBottom: messagesHook.scrollToBottom,
      setMessageBlocks: messagesHook.setMessageBlocks,
    });
  }, [coachSelection, conversations, messagesHook]);

  // Provider connection handling
  const handleConnectProvider = useCallback(async (provider: string) => {
    await providerStatus.handleConnectProvider(provider, async () => {
      if (coachSelection.pendingCoachAction) {
        await startCoachConversation(coachSelection.pendingCoachAction.coach);
        coachSelection.clearPendingCoachAction();
      } else if (pendingPrompt) {
        await handleSendPromptMessage(pendingPrompt);
        setPendingPrompt(null);
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providerStatus, coachSelection, pendingPrompt, startCoachConversation]);

  const handleSendPromptMessage = useCallback(async (prompt: string) => {
    let conversationId = conversations.currentConversation?.id;
    if (!conversationId) {
      try {
        const newConversation = await conversations.createConversation({
          title: prompt.slice(0, 50),
        });
        conversationId = newConversation.id;
      } catch {
        Alert.alert('Error', conversations.error || 'Failed to create conversation');
        return;
      }
    }
    await messagesHook.sendTurn(conversationId, prompt);
  }, [conversations, messagesHook]);

  // Provider modal handlers
  const handleProviderSelect = useCallback((provider: string) => {
    providerStatus.setSelectedProvider(provider);
    providerStatus.setProviderModalVisible(false);
    if (pendingPrompt) {
      handleSendPromptMessage(pendingPrompt);
      setPendingPrompt(null);
    }
    if (coachSelection.pendingCoachAction) {
      startCoachConversation(coachSelection.pendingCoachAction.coach);
      coachSelection.clearPendingCoachAction();
    }
  }, [providerStatus, pendingPrompt, coachSelection, handleSendPromptMessage, startCoachConversation]);

  const handleProviderModalClose = useCallback(() => {
    providerStatus.setProviderModalVisible(false);
    setPendingPrompt(null);
    coachSelection.clearPendingCoachAction();
  }, [providerStatus, coachSelection]);

  // Header menu handlers
  const showTitleActionMenu = useCallback(() => {
    if (!conversations.currentConversation) return;
    setActionMenuVisible(true);
  }, [conversations.currentConversation]);

  const handleMenuRename = useCallback(() => {
    setActionMenuVisible(false);
    if (conversations.currentConversation) {
      const title = conversations.currentConversation.title || 'New Chat';
      setRenameConversationId(conversations.currentConversation.id);
      setRenameDefaultTitle(title);
      setRenamePromptVisible(true);
    }
  }, [conversations.currentConversation]);

  const handleMenuParticipants = useCallback(() => {
    setActionMenuVisible(false);
    if (conversations.currentConversation) {
      chatPlus.flows.openParticipants();
    }
  }, [conversations.currentConversation, chatPlus.flows]);

  const handleMenuDelete = useCallback(() => {
    setActionMenuVisible(false);
    if (!conversations.currentConversation) return;

    Alert.alert(
      'Delete Conversation',
      `Are you sure you want to delete "${conversations.currentConversation.title || 'this conversation'}"?`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Delete',
          style: 'destructive',
          onPress: async () => {
            await conversations.deleteConversation(conversations.currentConversation!.id);
            // The thread is gone; the list is where the athlete goes next.
            goBackToList();
          },
        },
      ]
    );
  }, [conversations, goBackToList]);

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

  const isCoachConversation = Boolean(conversations.currentConversation?.coach_id);

  return (
    <View className="flex-1 bg-background-primary" testID="chat-screen">
      <View
        className="flex-1"
      >
        <ChatHeader
          currentConversation={conversations.currentConversation}
          actionMenuVisible={actionMenuVisible}
          insetTop={insets.top}
          onBackPress={goBackToList}
          onPlusPress={() => setPlusVisible(true)}
          onTitlePress={showTitleActionMenu}
          onMenuClose={() => setActionMenuVisible(false)}
          onMenuRename={handleMenuRename}
          onMenuParticipants={handleMenuParticipants}
          onMenuDelete={handleMenuDelete}
        />

        <ChatPlusSheet
          visible={plusVisible}
          onClose={() => setPlusVisible(false)}
          actions={chatPlus.actions}
        />
        <ChatPlusFlows flows={chatPlus.flows} />

        <MessageList
          messages={messagesHook.messages}
          coaches={coachSelection.coaches}
          isLoading={conversations.isLoading}
          isSending={messagesHook.isSending}
          isCoachConversation={isCoachConversation}
          messageFeedback={messagesHook.messageFeedback}
          messageFeedbackComment={messagesHook.messageFeedbackComment}
          messageBlocks={messagesHook.messageBlocks}
          verdicts={messagesHook.verdicts}
          flatListRef={messagesHook.flatListRef}
          onScrollToBottom={messagesHook.scrollToBottom}
          onCoachSelect={handleCoachSelect}
          onThumbsUp={handleThumbsUp}
          onThumbsDown={handleThumbsDown}
          onSubmitFeedbackReason={handleSubmitFeedbackReason}
          onRetryMessage={handleRetryMessage}
          onOpenUrl={handleOpenUrl}
          onActionClick={handleActionClick}
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
                  <Text className="text-base text-text-tertiary">Close</Text>
                </TouchableOpacity>
              </View>
            </View>
          </Modal>
        )}

        <PromptDialog
          visible={renamePromptVisible}
          title="Rename Chat"
          message="Enter a new name for this conversation"
          defaultValue={renameDefaultTitle}
          submitText="Save"
          cancelText="Cancel"
          onSubmit={handleRenameSubmit}
          onCancel={handleRenameCancel}
          testID="rename-conversation-dialog"
        />

      </View>
    </View>
  );
}
