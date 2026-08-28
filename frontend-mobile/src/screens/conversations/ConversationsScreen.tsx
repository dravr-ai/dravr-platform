// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The chat tab's landing screen — every conversation the athlete takes part in, one flat Telegram-shaped list
// ABOUTME: A row opens its thread; the "+" starts a chat or a group chat; search, swipe and long-press act on rows

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
import { SafeAreaView } from 'react-native-safe-area-context';
import { useFocusEffect, useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { LinearGradient } from 'expo-linear-gradient';
import { filterRows, type ConversationRowModel } from '@pierre/chat-utils';
import { glassCard, gradients, useThemeColors } from '../../constants/theme';
import { FloatingSearchBar, PromptDialog, TAB_BAR_BOTTOM_OFFSET } from '../../components/ui';
import { AppearanceToggleButton } from '../../components/ui/AppearanceToggleButton';
import { NotificationBellButton } from '../../components/notifications/NotificationBellButton';
import { threadHref } from '../../navigation/routes';
import { ChatPlusSheet } from '../chat/ChatPlusSheet';
import { ChatPlusFlows } from '../chat/ChatPlusFlows';
import { useChatPlusActions } from '../chat/useChatPlusActions';
import { ConversationRow } from './ConversationRow';
import { useConversationList } from './useConversationList';
import { useTranslation } from '@pierre/i18n';

// Glassmorphic menu style
const menuStyle: ViewStyle = {
  ...glassCard,
  borderRadius: 16,
  borderColor: 'rgba(139, 92, 246, 0.2)',
};

/** What the list says when the athlete has no conversation yet. */
export const EMPTY_LIST_LINE = 'No chats yet — start one with the +';

function describeError(err: unknown, fallback: string): string {
  return err instanceof Error ? err.message : fallback;
}

export function ConversationsScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const router = useRouter();
  const list = useConversationList();
  const [searchQuery, setSearchQuery] = useState('');
  const [actionMenuVisible, setActionMenuVisible] = useState(false);
  const [selectedRow, setSelectedRow] = useState<ConversationRowModel | null>(null);
  const [renamePromptVisible, setRenamePromptVisible] = useState(false);
  const [plusVisible, setPlusVisible] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // No conversation is open on the list, so the "+" offers new chat and new
  // group chat; "add someone" belongs to the thread that is being read.
  const chatPlus = useChatPlusActions(null);

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
          setActionError(describeError(err, 'Failed to mark conversation read'));
        });
      }
    },
    [router, list],
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
        setActionError(describeError(err, 'Failed to mark conversation unread'));
      });
    },
    [list],
  );

  const confirmDelete = useCallback(
    (row: ConversationRowModel) => {
      Alert.alert(t('app.convDeleteTitle'), `Are you sure you want to delete "${row.title}"?`, [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('common.delete'),
          style: 'destructive',
          onPress: () => {
            list.remove(row.id).catch((err: unknown) => {
              setActionError(describeError(err, 'Failed to delete conversation'));
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
        setActionError(describeError(err, 'Failed to rename conversation'));
      });
    },
    [selectedRow, list],
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

  const errorMessage = actionError ?? (list.isError ? describeError(list.error, 'Failed to load conversations') : null);

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="conversations-screen">
      {/* Header — the landing screen's chrome: title, appearance, bell, and the chat "+" */}
      <View className="flex-row items-center px-4 py-2 border-b border-border-subtle">
        <Text className="flex-1 text-xl font-bold text-text-primary" testID="conversations-title">
          {t('app.convListTitle')}
        </Text>
        <AppearanceToggleButton size={20} color={colors.text.secondary} />
        <NotificationBellButton size={20} color={colors.text.secondary} />
        <TouchableOpacity
          className="w-10 h-10 items-center justify-center"
          onPress={() => setPlusVisible(true)}
          accessibilityRole="button"
          accessibilityLabel={t('app.convNewAria')}
          testID="chat-plus-button"
        >
          <Feather name="plus" size={24} color={colors.pierre.violet} />
        </TouchableOpacity>
      </View>

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
          contentContainerStyle={{ paddingBottom: TAB_BAR_BOTTOM_OFFSET + 64 }}
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
                  <Text className="text-base text-text-secondary text-center">{EMPTY_LIST_LINE}</Text>
                  <TouchableOpacity
                    className="w-12 h-12 rounded-full items-center justify-center mt-4"
                    style={{ backgroundColor: `${colors.pierre.violet}26` }}
                    onPress={() => setPlusVisible(true)}
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

      {/* Floating search bar — sits above the tab bar, rides the keyboard */}
      <FloatingSearchBar
        value={searchQuery}
        onChangeText={setSearchQuery}
        placeholder={t('app.convSearchPlaceholder')}
        testID="conversation-search-input"
      />

      <ChatPlusSheet visible={plusVisible} onClose={() => setPlusVisible(false)} actions={chatPlus.actions} />
      <ChatPlusFlows flows={chatPlus.flows} />

      {/* Long-press menu */}
      <Modal visible={actionMenuVisible} animationType="fade" transparent onRequestClose={closeActionMenu}>
        <TouchableOpacity className="flex-1 bg-black/50 justify-center items-center" activeOpacity={1} onPress={closeActionMenu}>
          <View className="min-w-[240px] overflow-hidden" style={menuStyle} testID="conversation-action-menu">
            <LinearGradient
              colors={gradients.violetCyan as [string, string]}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ height: 3, width: '100%' }}
            />
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
        submitText="Save"
        cancelText={t('common.cancel')}
        onSubmit={handleRenameSubmit}
        onCancel={handleRenameCancel}
        testID="rename-conversation-dialog"
      />
    </SafeAreaView>
  );
}
