// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The chat tab's landing screen — every conversation the athlete takes part in, one flat Telegram-shaped list
// ABOUTME: A row opens its thread; the header "+" starts a chat or a group chat; the native search, swipe and long-press act on rows

import React, { useCallback, useMemo, useRef, useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  Modal,
  type ViewStyle,
} from 'react-native';
import { FlashList } from '@shopify/flash-list';
import { Stack, useFocusEffect, useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { filterRows, type ConversationRowModel } from '@pierre/chat-utils';
import { spacing, useCardStyle, useThemeColors } from '../../constants/theme';
import { BrandLockup, PromptDialog } from '../../components/ui';
import { AppearanceToggleButton } from '../../components/ui/AppearanceToggleButton';
import { HeaderActions } from '../../components/ui/HeaderActions';
import { NotificationBellButton } from '../../components/notifications/NotificationBellButton';
import { threadHref } from '../../navigation/routes';
import { ChatPlusFlows } from '../chat/ChatPlusFlows';
import { NewChatButton } from '../chat/NewChatButton';
import { presentChatPlusMenu } from '../chat/presentChatPlusMenu';
import { useChatPlusActions } from '../chat/useChatPlusActions';
import { ConversationRow } from './ConversationRow';
import { useConversationList } from './useConversationList';
import { useTranslation } from '@pierre/i18n';

function describeError(err: unknown, fallback: string): string {
  return err instanceof Error ? err.message : fallback;
}

export function ConversationsScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  // The long-press menu wears the resting-card recipe, but takes the strong
  // hairline rather than the recipe's default: it floats over a scrim instead
  // of sitting on the canvas, and in dark its fill separates from the dimmed
  // list beneath by only 1.38:1, so the edge is what draws the sheet.
  const menuStyle: ViewStyle = {
    ...useCardStyle(),
    borderRadius: 16,
    borderColor: colors.border.strong,
  };
  const router = useRouter();
  const list = useConversationList();
  const [searchQuery, setSearchQuery] = useState('');
  const [actionMenuVisible, setActionMenuVisible] = useState(false);
  const [selectedRow, setSelectedRow] = useState<ConversationRowModel | null>(null);
  const [renamePromptVisible, setRenamePromptVisible] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // No conversation is open on the list, so the "+" offers new chat and new
  // group chat; "add someone" belongs to the thread that is being read.
  const chatPlus = useChatPlusActions(null);
  const openPlusMenu = useCallback(
    () =>
      presentChatPlusMenu({
        actions: chatPlus.actions,
        cancelLabel: t('common.cancel'),
        title: t('app.convNewAria'),
      }),
    [chatPlus.actions, t],
  );

  // The query fetches on mount; a focus after that — coming back from a
  // thread, from Discover, from a notification — re-reads the list so a row
  // that moved or was read elsewhere is drawn where it belongs.
  const focusedOnce = useRef(false);
  const { refetch } = list;
  useFocusEffect(
    useCallback(() => {
      if (!focusedOnce.current) {
        focusedOnce.current = true;
        return;
      }
      void refetch();
    }, [refetch]),
  );

  const visibleRows = useMemo(() => filterRows(list.rows, searchQuery), [list.rows, searchQuery]);

  const openThread = useCallback(
    (row: ConversationRowModel) => {
      router.push(threadHref(row.id));
      // Opening a thread reads it. The marker only moves when the row has
      // something unread — advancing it is monotonic server-side anyway.
      if (row.unreadCount > 0) {
        list.markRead(row.id).catch((err: unknown) => {
          setActionError(describeError(err, t('app.failedMarkRead')));
        });
      }
    },
    [router, list, t],
  );

  const openActionMenu = useCallback((row: ConversationRowModel) => {
    setSelectedRow(row);
    setActionMenuVisible(true);
  }, []);

  const closeActionMenu = useCallback(() => {
    setActionMenuVisible(false);
    setSelectedRow(null);
  }, []);

  const markUnread = useCallback(
    (row: ConversationRowModel) => {
      list.markUnread(row.id).catch((err: unknown) => {
        setActionError(describeError(err, t('app.failedMarkUnread')));
      });
    },
    [list, t],
  );

  const confirmDelete = useCallback(
    (row: ConversationRowModel) => {
      Alert.alert(t('app.convDeleteTitle'), t('app.confirmDeleteConversation', { title: row.title }), [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('common.delete'),
          style: 'destructive',
          onPress: () => {
            list.remove(row.id).catch((err: unknown) => {
              setActionError(describeError(err, t('app.failedDeleteConversation')));
            });
          },
        },
      ]);
    },
    [list, t],
  );

  const handleMenuRename = useCallback(() => {
    if (!selectedRow) return;
    setActionMenuVisible(false);
    setRenamePromptVisible(true);
  }, [selectedRow]);

  const handleMenuMarkUnread = useCallback(() => {
    if (!selectedRow) return;
    const row = selectedRow;
    closeActionMenu();
    markUnread(row);
  }, [selectedRow, closeActionMenu, markUnread]);

  const handleMenuDelete = useCallback(() => {
    if (!selectedRow) return;
    const row = selectedRow;
    closeActionMenu();
    confirmDelete(row);
  }, [selectedRow, closeActionMenu, confirmDelete]);

  const handleRenameSubmit = useCallback(
    (newTitle: string) => {
      setRenamePromptVisible(false);
      if (!selectedRow) return;
      const row = selectedRow;
      setSelectedRow(null);
      list.rename(row.id, newTitle).catch((err: unknown) => {
        setActionError(describeError(err, t('app.failedRenameConversation')));
      });
    },
    [selectedRow, list, t],
  );

  const handleRenameCancel = useCallback(() => {
    setRenamePromptVisible(false);
    setSelectedRow(null);
  }, []);

  const renderRow = useCallback(
    ({ item }: { item: ConversationRowModel }) => (
      <ConversationRow
        row={item}
        onPress={openThread}
        onLongPress={openActionMenu}
        onMarkUnread={markUnread}
        onDelete={confirmDelete}
      />
    ),
    [openThread, openActionMenu, markUnread, confirmDelete],
  );

  const keyExtractor = useCallback((item: ConversationRowModel) => item.id, []);

  const errorMessage = actionError ?? (list.isError ? describeError(list.error, t('app.failedLoadConversations')) : null);

  return (
    <View className="flex-1 bg-background-primary" testID="conversations-screen">
      {/*
        The native header carries the landing screen's chrome: the lockup as
        its title view, then appearance, the bell and the chat "+"; the search
        field is the system's, under the bar on iOS 18 and in the bottom
        toolbar on iOS 26.

        The lockup stands where the screen title would, the way every
        messenger writes its own name across the top of its conversation list.
        The chat tab is the only one that carries it (DESIGN.md §5): the phone
        has no icon rail to hold the mark, so this header is the single place
        the athlete's Dravr identity can live, and repeating it on every tab
        would make it chrome instead. The destination's name stays the spoken
        one, so a screen reader still says which tab this is.
      */}
      <Stack.Screen
        options={{
          headerTitle: () => (
            <BrandLockup accessibilityLabel={t('app.convListTitle')} testID="conversations-title" />
          ),
          headerRight: () => (
            <HeaderActions>
              <AppearanceToggleButton size={20} color={colors.text.secondary} />
              <NotificationBellButton size={20} color={colors.text.secondary} />
              <NewChatButton actions={chatPlus.actions} />
            </HeaderActions>
          ),
          headerSearchBarOptions: {
            placeholder: t('app.convSearchPlaceholder'),
            autoCapitalize: 'none',
            hideWhenScrolling: false,
            onChangeText: (event) => setSearchQuery(event.nativeEvent.text),
          },
        }}
      />

      {errorMessage && (
        <View
          className="mx-3 mt-2 p-3 bg-error/10 border border-error/30 rounded-lg flex-row items-center justify-between"
          testID="conversations-error"
        >
          <Text className="flex-1 text-error text-sm mr-3">{errorMessage}</Text>
          <TouchableOpacity
            className="px-3 py-1.5 bg-error/20 rounded-md"
            onPress={() => {
              setActionError(null);
              void list.refetch();
            }}
            testID="conversations-retry"
          >
            <Text className="text-error text-sm font-semibold">{t('common.retry')}</Text>
          </TouchableOpacity>
        </View>
      )}

      {list.isLoading ? (
        <View className="flex-1 items-center justify-center" testID="conversations-loading">
          <ActivityIndicator size="large" color={colors.pierre.violet} />
        </View>
      ) : (
        <FlashList
          data={visibleRows}
          renderItem={renderRow}
          keyExtractor={keyExtractor}
          // The system header, search field and tab bar inset the list themselves.
          contentInsetAdjustmentBehavior="automatic"
          contentContainerStyle={{ paddingBottom: spacing.lg }}
          showsVerticalScrollIndicator={false}
          keyboardShouldPersistTaps="handled"
          onEndReached={list.loadMore}
          onEndReachedThreshold={0.5}
          refreshing={list.isRefetching}
          onRefresh={() => void list.refetch()}
          ListFooterComponent={
            list.isLoadingMore ? (
              <View className="py-4 items-center" testID="conversations-loading-more">
                <ActivityIndicator size="small" color={colors.pierre.violet} />
              </View>
            ) : null
          }
          testID="conversations-list"
          ListEmptyComponent={
            <View className="flex-1 items-center justify-center pt-16 px-8" testID="conversations-empty">
              {searchQuery.trim() ? (
                <Text className="text-base text-text-secondary text-center">
                  {t('app.convNoSearchMatch', { query: searchQuery.trim() })}
                </Text>
              ) : (
                <>
                  <Text className="text-base text-text-secondary text-center">{t('chat.noChatsEmptyMobile')}</Text>
                  <TouchableOpacity
                    className="w-12 h-12 rounded-full items-center justify-center mt-4"
                    style={{ backgroundColor: `${colors.pierre.violet}26` }}
                    onPress={openPlusMenu}
                    accessibilityRole="button"
                    accessibilityLabel={t('app.convNewAria')}
                    testID="conversations-empty-plus"
                  >
                    <Feather name="plus" size={24} color={colors.pierre.violet} />
                  </TouchableOpacity>
                </>
              )}
            </View>
          }
        />
      )}

      <ChatPlusFlows flows={chatPlus.flows} />

      {/* Long-press menu */}
      <Modal visible={actionMenuVisible} animationType="fade" transparent onRequestClose={closeActionMenu}>
        <TouchableOpacity className="flex-1 bg-scrim/60 justify-center items-center" activeOpacity={1} onPress={closeActionMenu}>
          <View className="min-w-[240px] overflow-hidden" style={menuStyle} testID="conversation-action-menu">
            <View className="py-2">
              <TouchableOpacity
                className="flex-row items-center px-4 py-3"
                onPress={handleMenuRename}
                testID="conversation-action-rename"
              >
                <Feather name="edit-2" size={18} color={colors.text.primary} />
                <Text className="text-base text-text-primary ml-3">{t('app.convMenuRename')}</Text>
              </TouchableOpacity>

              <TouchableOpacity
                className="flex-row items-center px-4 py-3"
                onPress={handleMenuMarkUnread}
                testID="conversation-action-mark-unread"
              >
                <Feather name="mail" size={18} color={colors.text.primary} />
                <Text className="text-base text-text-primary ml-3">{t('app.convMenuMarkUnread')}</Text>
              </TouchableOpacity>

              <TouchableOpacity
                className="flex-row items-center px-4 py-3"
                onPress={handleMenuDelete}
                testID="conversation-action-delete"
              >
                <Feather name="trash-2" size={18} color={colors.error} />
                <Text className="text-base text-error ml-3">{t('common.delete')}</Text>
              </TouchableOpacity>
            </View>
          </View>
        </TouchableOpacity>
      </Modal>

      <PromptDialog
        visible={renamePromptVisible}
        title={t('app.convRenameTitle')}
        message="Enter a new name for this conversation"
        defaultValue={selectedRow?.title ?? ''}
        submitText={t('common.save')}
        cancelText={t('common.cancel')}
        onSubmit={handleRenameSubmit}
        onCancel={handleRenameCancel}
        testID="rename-conversation-dialog"
      />
    </View>
  );
}
