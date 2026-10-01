// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Main chat screen orchestrator importing decomposed hooks and components
// ABOUTME: Resolves which conversation is open and how a turn opens one; the shared ChatThread draws it

import React, { useState, useRef, useEffect, useCallback, useMemo } from 'react';
import {
  View,
  Text,
  TextInput,
  Alert,
  AppState,
  type AppStateStatus,
} from 'react-native';
import { KeyboardAvoidingView } from 'react-native-keyboard-controller';
import { useHeaderHeight } from 'expo-router/react-navigation';
import { Stack, useRouter, useLocalSearchParams, useFocusEffect } from 'expo-router';

import { useAuth } from '../../contexts/AuthContext';
import { HeaderActions, PromptDialog } from '../../components/ui';
import { AppearanceToggleButton } from '../../components/ui/AppearanceToggleButton';
import { NotificationBellButton } from '../../components/notifications/NotificationBellButton';
import { useThemeColors } from '../../constants/theme';
import { trackMobile } from '../../services/analytics';
import { providerStatusLine } from '@pierre/chat-utils';

import { ChatHeaderTitle } from './ChatHeaderTitle';
import { ChatPlusFlows } from './ChatPlusFlows';
import { useChatPlusActions } from './useChatPlusActions';
import { CHAT_LIST_ROUTE, NEW_CONVERSATION_ID, threadHref } from '../../navigation/routes';
import { ChatThread } from './ChatThread';
import { ConversationInfoSheet } from './ConversationInfoSheet';
import { ReconnectBanner } from '../../components/ReconnectBanner';
import { useProviderConnected } from '../../hooks/useHome';
import { useConversations } from './useConversations';
import { useMarkConversationRead } from './useMarkConversationRead';
import { useMessages } from './useMessages';
import { useProviderStatus } from './useProviderStatus';
import { useUsageStatus } from './useUsageStatus';
import { useTranslation } from '@pierre/i18n';

export function ChatScreen() {
  const { t } = useTranslation();
  const { isAuthenticated } = useAuth();
  // The native header sits above this screen, so the keyboard-avoiding column
  // offsets by its height on iOS, where `padding` measures from the window.
  const headerHeight = useHeaderHeight();
  const colors = useThemeColors();
  const router = useRouter();
  const params = useLocalSearchParams<{ conversationId?: string; draft?: string; send?: string }>();
  const inputRef = useRef<TextInput>(null);

  // UI State
  const [inputText, setInputText] = useState('');
  const [infoVisible, setInfoVisible] = useState(false);
  const [renamePromptVisible, setRenamePromptVisible] = useState(false);
  const [renameConversationId, setRenameConversationId] = useState<string | null>(null);
  const [renameDefaultTitle, setRenameDefaultTitle] = useState('');

  // Custom hooks
  const conversations = useConversations();
  const messagesHook = useMessages();
  const providerStatus = useProviderStatus();
  // The header's fallback line reads the status the reconnect banner under it
  // reads, so the two can never disagree about a flagged connection.
  const { providers: statusRows } = useProviderConnected();
  const headerProviderStatus = useMemo(() => providerStatusLine(t, statusRows, statusRows !== null), [statusRows, t]);
  const usageStatus = useUsageStatus();
  // The flow state behind the info sheet's "Participants" row. The tab bar's
  // "+" holds its own copy for the same thread, so "add someone to this
  // discussion" and "Participants" open the same control either way.
  const chatPlus = useChatPlusActions(conversations.currentConversation?.id ?? null);

  const { messages } = messagesHook;
  // The newest row of the caller's own conversation: the read marker names a
  // chat message, and a group thread's room rows are not the caller's.
  const lastMessageId = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      if (!messages[i].room) return messages[i].id;
    }
    return null;
  }, [messages]);
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
          void messagesHook.loadMessages(openId, conversations.currentConversation?.group_id);
        }
      }
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [isAuthenticated, conversations.currentConversation?.id, messagesHook.isSending])
  );

  // Coming back to the app is coming back to the thread: another member's
  // line, or a reply sent from a channel, landed with no push to the app, so
  // the open thread — a group thread's room included — is read again on the
  // way back to the foreground. Skipped mid-send, as the focus read is.
  const openConversationId = conversations.currentConversation?.id;
  const openGroupId = conversations.currentConversation?.group_id;
  const { isSending, loadMessages } = messagesHook;
  useEffect(() => {
    if (!isAuthenticated || !openConversationId) return undefined;
    const subscription = AppState.addEventListener('change', (status: AppStateStatus) => {
      if (status === 'active' && !isSending) {
        void loadMessages(openConversationId, openGroupId);
      }
    });
    return () => subscription.remove();
  }, [isAuthenticated, openConversationId, openGroupId, isSending, loadMessages]);

  // Load messages when conversation changes
  useEffect(() => {
    if (conversations.currentConversation) {
      if (conversations.justCreatedConversationRef.current === conversations.currentConversation.id) {
        conversations.justCreatedConversationRef.current = null;
        return;
      }
      messagesHook.loadMessages(
        conversations.currentConversation.id,
        conversations.currentConversation.group_id,
      );
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
      // The server names the thread — after its agent when one is bound,
      // else for the moment it starts in the athlete's language — so the
      // stored title is the one both clients print. The first line used to
      // become the title, so a thread was named after whatever was typed into
      // it and a rename had to undo that.
      const newConversation = await conversations.createConversation({});
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
      {/*
        The list, the progress strip, the usage banner and the composer bar
        are one column; the keyboard shortens it through the layout. Both
        platforms pad: Android runs edge-to-edge, where the window no longer
        resizes for the keyboard, so this is the keyboard-controller view
        that reads the IME inset itself (the same tracker the auth forms use)
        rather than React Native's, which the composer sat behind the
        keyboard under. The native header is above this column on both
        platforms, so both offset by its height.
      */}
      <KeyboardAvoidingView
        style={{ flex: 1 }}
        behavior="padding"
        keyboardVerticalOffset={headerHeight}
      >
        {/*
          The native header: the system back chevron, the thread's avatar and
          title as the title view, appearance and the bell. Nothing follows the
          bell — starting a discussion belongs to the list's "+" (carnet#213).
        */}
        <Stack.Screen
          options={{
            headerTitle: () => (
              <ChatHeaderTitle
                currentConversation={conversations.currentConversation}
                providerStatus={headerProviderStatus}
                onTitlePress={openInfoSheet}
              />
            ),
            headerRight: () => (
              <HeaderActions>
                <AppearanceToggleButton size={20} color={colors.text.secondary} />
                <NotificationBellButton size={20} color={colors.text.secondary} />
              </HeaderActions>
            ),
          }}
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

        {/*
          The thread is pushed over the tabs, so the shell's banner is behind
          it: the thread mounts its own, under the native header, which
          already sits below the status bar — so no inset of its own.
        */}
        <ReconnectBanner insetTop={false} />

        {messagesHook.roomUnavailable ? (
          <Text
            testID="room-load-failed"
            className="text-sm text-text-tertiary text-center px-4 pt-2"
          >
            {t('groups.roomLoadFailed')}
          </Text>
        ) : null}

        <ChatThread
          conversationId={conversations.currentConversation?.id ?? null}
          messagesHook={messagesHook}
          usageStatus={usageStatus}
          providerStatus={providerStatus}
          isLoading={conversations.isLoading}
          inputText={inputText}
          onChangeInputText={setInputText}
          inputRef={inputRef}
          sendText={sendText}
        />

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
      </KeyboardAvoidingView>
    </View>
  );
}
