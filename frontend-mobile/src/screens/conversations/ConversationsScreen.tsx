// ABOUTME: Full conversations list screen with search and coach-session grouping
// ABOUTME: Groups conversations by coach_id with collapsible sections (mobile parity with web ConversationsPanel)

import React, { useState, useCallback, useEffect, useMemo } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  TextInput,
  ActivityIndicator,
  Alert,
  Modal,
  type ViewStyle,
} from 'react-native';
import { FlashList } from '@shopify/flash-list';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useFocusEffect } from 'expo-router';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { LinearGradient } from 'expo-linear-gradient';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { colors, spacing, glassCard, gradients, buttonGlow } from '../../constants/theme';
import { chatApi, coachesApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { PromptDialog, SwipeableRow, type SwipeAction } from '../../components/ui';
import type { Conversation, Coach } from '../../types';

// Glassmorphic search bar style
const searchBarStyle: ViewStyle = {
  ...glassCard,
  borderRadius: 22,
  borderColor: 'rgba(139, 92, 246, 0.2)',
};

// FAB with violet glow
const fabStyle: ViewStyle = {
  backgroundColor: colors.pierre.violet,
  ...buttonGlow,
};

// Glassmorphic menu style
const menuStyle: ViewStyle = {
  ...glassCard,
  borderRadius: 16,
  borderColor: 'rgba(139, 92, 246, 0.2)',
};

/**
 * A conversation session grouping: one bucket per coach_id, plus one
 * "no coach" bucket for unattached conversations. Mirrors the web
 * ConversationsPanel session-hierarchy model (Sprint C15).
 */
interface SessionGroup {
  key: string;
  label: string;
  conversations: Conversation[];
  isNoCoach: boolean;
}

const COLLAPSED_KEY = 'dravr.conversations-panel.collapsed';
const NO_COACH_KEY = '__no_coach__';

type ListRow =
  | { kind: 'header'; group: SessionGroup; isCollapsed: boolean }
  | { kind: 'conversation'; conversation: Conversation; groupKey: string };

export function ConversationsScreen() {
  const router = useRouter();
  const { isAuthenticated } = useAuth();
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [coaches, setCoaches] = useState<Coach[]>([]);
  const [searchQuery, setSearchQuery] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [actionMenuVisible, setActionMenuVisible] = useState(false);
  const [selectedConversation, setSelectedConversation] = useState<Conversation | null>(null);
  const [renamePromptVisible, setRenamePromptVisible] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());

  // Hydrate collapsed state from AsyncStorage once on mount.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const raw = await AsyncStorage.getItem(COLLAPSED_KEY);
        if (cancelled || !raw) return;
        const parsed = JSON.parse(raw) as unknown;
        if (Array.isArray(parsed)) {
          const onlyStrings = parsed.filter((v): v is string => typeof v === 'string');
          setCollapsedGroups(new Set(onlyStrings));
        }
      } catch {
        // Non-fatal — keep empty set if storage is corrupt or unavailable.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const persistCollapsed = useCallback((next: Set<string>): void => {
    AsyncStorage.setItem(COLLAPSED_KEY, JSON.stringify(Array.from(next))).catch(() => {
      // Non-fatal — the UI stays consistent within the session.
    });
  }, []);

  const toggleGroup = useCallback(
    (groupKey: string): void => {
      setCollapsedGroups((prev) => {
        const next = new Set(prev);
        if (next.has(groupKey)) {
          next.delete(groupKey);
        } else {
          next.add(groupKey);
        }
        persistCollapsed(next);
        return next;
      });
    },
    [persistCollapsed]
  );

  const loadConversations = useCallback(async () => {
    if (!isAuthenticated) return;

    try {
      setIsLoading(true);
      setError(null);
      const [convResponse, coachResponse] = await Promise.all([
        chatApi.getConversations(),
        coachesApi.list().catch(() => ({ coaches: [] as Coach[] })),
      ]);
      const seen = new Set<string>();
      const deduplicated = (convResponse.conversations || []).filter((conv: { id: string }) => {
        if (seen.has(conv.id)) return false;
        seen.add(conv.id);
        return true;
      });
      const sorted = deduplicated.sort(
        (a: { updated_at: string }, b: { updated_at: string }) =>
          new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime()
      );
      setConversations(sorted);
      setCoaches(coachResponse.coaches || []);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to load conversations';
      setError(errorMessage);
      console.error('Failed to load conversations:', err);
    } finally {
      setIsLoading(false);
    }
  }, [isAuthenticated]);

  useFocusEffect(
    useCallback(() => {
      loadConversations();
    }, [loadConversations])
  );

  const coachTitleById = useMemo(() => {
    const map = new Map<string, string>();
    for (const c of coaches) {
      map.set(c.id, c.title);
    }
    return map;
  }, [coaches]);

  // Apply search filter first — searches across all groups so a matching
  // conversation in a collapsed group still surfaces.
  const filteredConversations = useMemo(() => {
    const trimmed = searchQuery.trim();
    if (!trimmed) return conversations;
    const query = trimmed.toLowerCase();
    return conversations.filter((conv) => (conv.title || '').toLowerCase().includes(query));
  }, [searchQuery, conversations]);

  const sessionGroups = useMemo<SessionGroup[]>(() => {
    const buckets = new Map<string, Conversation[]>();
    for (const conv of filteredConversations) {
      const key = conv.coach_id ?? NO_COACH_KEY;
      const bucket = buckets.get(key);
      if (bucket) {
        bucket.push(conv);
      } else {
        buckets.set(key, [conv]);
      }
    }

    const groups: SessionGroup[] = [];
    for (const [key, convs] of buckets.entries()) {
      if (key === NO_COACH_KEY) continue;
      groups.push({
        key,
        label: coachTitleById.get(key) ?? 'Unknown coach',
        conversations: convs,
        isNoCoach: false,
      });
    }
    // Sort coach sessions by the most recent conversation's updated_at desc.
    groups.sort((a, b) => {
      const aLast = a.conversations[0]?.updated_at ?? '';
      const bLast = b.conversations[0]?.updated_at ?? '';
      return bLast.localeCompare(aLast);
    });

    const noCoachBucket = buckets.get(NO_COACH_KEY);
    if (noCoachBucket && noCoachBucket.length > 0) {
      groups.push({
        key: NO_COACH_KEY,
        label: 'Without a coach',
        conversations: noCoachBucket,
        isNoCoach: true,
      });
    }

    return groups;
  }, [filteredConversations, coachTitleById]);

  // Flatten groups into a single FlashList-friendly row list.
  // When a search filter is active we force-expand every group so matches
  // stay visible even if the user had that coach collapsed.
  const listRows = useMemo<ListRow[]>(() => {
    const searchActive = searchQuery.trim().length > 0;
    const rows: ListRow[] = [];
    for (const group of sessionGroups) {
      const isCollapsed = !searchActive && collapsedGroups.has(group.key);
      rows.push({ kind: 'header', group, isCollapsed });
      if (!isCollapsed) {
        for (const conv of group.conversations) {
          rows.push({ kind: 'conversation', conversation: conv, groupKey: group.key });
        }
      }
    }
    return rows;
  }, [sessionGroups, collapsedGroups, searchQuery]);

  const formatRelativeDate = (dateString: string): string => {
    const date = new Date(dateString);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));
    const diffWeeks = Math.floor(diffDays / 7);
    const diffMonths = Math.floor(diffDays / 30);

    if (diffDays === 0) {
      return 'today';
    } else if (diffDays === 1) {
      return 'yesterday';
    } else if (diffDays < 7) {
      return `${diffDays} days ago`;
    } else if (diffWeeks === 1) {
      return '1 week ago';
    } else if (diffWeeks < 4) {
      return `${diffWeeks} weeks ago`;
    } else if (diffMonths === 1) {
      return '1 month ago';
    } else {
      return `${diffMonths} months ago`;
    }
  };

  const handleConversationPress = (conversationId: string) => {
    router.push({ pathname: '/(app)/(tabs)/(chat)', params: { conversationId } });
  };

  const handleConversationLongPress = (conversation: Conversation) => {
    setSelectedConversation(conversation);
    setActionMenuVisible(true);
  };

  const handleNewChat = () => {
    router.push('/(app)/(tabs)/(chat)');
  };

  const handleRename = () => {
    if (!selectedConversation) return;
    setActionMenuVisible(false);
    setRenamePromptVisible(true);
  };

  const handleRenameSubmit = async (newTitle: string) => {
    setRenamePromptVisible(false);
    if (!selectedConversation) return;

    try {
      const updated = await chatApi.updateConversation(selectedConversation.id, {
        title: newTitle,
      });
      setConversations((prev) =>
        prev.map((c) => (c.id === selectedConversation.id ? { ...c, title: updated.title } : c))
      );
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to rename conversation';
      setError(errorMessage);
      console.error('Failed to rename conversation:', err);
    } finally {
      setSelectedConversation(null);
    }
  };

  const handleRenameCancel = () => {
    setRenamePromptVisible(false);
    setSelectedConversation(null);
  };

  const handleDelete = () => {
    if (!selectedConversation) return;
    setActionMenuVisible(false);

    Alert.alert(
      'Delete Conversation',
      `Are you sure you want to delete "${selectedConversation.title || 'this conversation'}"?`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Delete',
          style: 'destructive',
          onPress: async () => {
            try {
              await chatApi.deleteConversation(selectedConversation.id);
              setConversations((prev) => prev.filter((c) => c.id !== selectedConversation.id));
            } catch (err) {
              const errorMessage = err instanceof Error ? err.message : 'Failed to delete conversation';
              setError(errorMessage);
              console.error('Failed to delete conversation:', err);
            }
          },
        },
      ]
    );
  };

  const closeActionMenu = () => {
    setActionMenuVisible(false);
    setSelectedConversation(null);
  };

  const renderGroupHeader = (group: SessionGroup, isCollapsed: boolean) => (
    <TouchableOpacity
      className="flex-row items-center px-4 py-3 bg-background-secondary/30 border-b border-border-subtle"
      onPress={() => toggleGroup(group.key)}
      accessibilityRole="button"
      accessibilityState={{ expanded: !isCollapsed }}
      accessibilityLabel={`Toggle ${group.label} session`}
      testID={`session-group-header-${group.key}`}
    >
      <Feather
        name={isCollapsed ? 'chevron-right' : 'chevron-down'}
        size={16}
        color={colors.text.tertiary}
      />
      <Feather
        name="message-square"
        size={14}
        color={group.isNoCoach ? colors.text.tertiary : colors.pierre.violet}
        style={{ marginLeft: 8 }}
      />
      <Text
        className={
          group.isNoCoach
            ? 'flex-1 text-sm text-text-tertiary ml-2'
            : 'flex-1 text-sm font-semibold text-text-primary ml-2'
        }
        numberOfLines={1}
      >
        {group.label}
      </Text>
      <Text className="text-xs text-text-tertiary ml-2">{group.conversations.length}</Text>
    </TouchableOpacity>
  );

  const renderConversationRow = (conversation: Conversation) => {
    const leftActions: SwipeAction[] = [
      {
        icon: 'edit-2',
        label: 'Rename',
        color: '#FFFFFF',
        backgroundColor: colors.pierre.violet,
        onPress: () => {
          setSelectedConversation(conversation);
          setRenamePromptVisible(true);
        },
      },
    ];

    const rightActions: SwipeAction[] = [
      {
        icon: 'trash-2',
        label: 'Delete',
        color: '#FFFFFF',
        backgroundColor: '#EF4444',
        onPress: () => {
          setSelectedConversation(conversation);
          Alert.alert(
            'Delete Conversation',
            `Are you sure you want to delete "${conversation.title || 'this conversation'}"?`,
            [
              { text: 'Cancel', style: 'cancel' },
              {
                text: 'Delete',
                style: 'destructive',
                onPress: async () => {
                  try {
                    await chatApi.deleteConversation(conversation.id);
                    setConversations((prev) => prev.filter((c) => c.id !== conversation.id));
                  } catch (err) {
                    const errorMessage = err instanceof Error ? err.message : 'Failed to delete conversation';
                    setError(errorMessage);
                    console.error('Failed to delete conversation:', err);
                  }
                },
              },
            ]
          );
        },
      },
    ];

    return (
      <SwipeableRow
        leftActions={leftActions}
        rightActions={rightActions}
        testID={`swipeable-conversation-${conversation.id}`}
      >
        <TouchableOpacity
          className="flex-row items-center pl-10 pr-4 py-3 border-b border-border-subtle bg-background-primary"
          onPress={() => handleConversationPress(conversation.id)}
          onLongPress={() => handleConversationLongPress(conversation)}
          delayLongPress={300}
        >
          <View className="flex-1">
            <Text className="text-base font-medium text-text-primary mb-0.5" numberOfLines={1}>
              {conversation.title || 'Untitled'}
            </Text>
            <Text className="text-sm text-text-tertiary">
              {formatRelativeDate(conversation.updated_at)}
            </Text>
          </View>
          <Text className="text-xl text-text-tertiary ml-2">›</Text>
        </TouchableOpacity>
      </SwipeableRow>
    );
  };

  const renderRow = ({ item }: { item: ListRow }) => {
    if (item.kind === 'header') {
      return renderGroupHeader(item.group, item.isCollapsed);
    }
    return renderConversationRow(item.conversation);
  };

  const keyExtractor = (item: ListRow) => {
    if (item.kind === 'header') return `header-${item.group.key}`;
    return `conv-${item.conversation.id}`;
  };

  return (
    <SafeAreaView className="flex-1 bg-background-primary">
      {/* Header */}
      <View className="flex-row items-center px-3 py-2 border-b border-border-subtle">
        <TouchableOpacity
          className="w-10 h-10 items-center justify-center"
          onPress={() => router.back()}
          testID="back-button"
        >
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-semibold text-text-primary text-center">
          Coaching sessions
        </Text>
        <View className="w-10" />
      </View>

      {/* Error Display */}
      {error && (
        <View className="mx-3 mt-2 p-3 bg-error/10 border border-error/30 rounded-lg flex-row items-center justify-between">
          <Text className="flex-1 text-error text-sm mr-3">{error}</Text>
          <TouchableOpacity
            className="px-3 py-1.5 bg-error/20 rounded-md"
            onPress={() => {
              setError(null);
              loadConversations();
            }}
          >
            <Text className="text-error text-sm font-semibold">Retry</Text>
          </TouchableOpacity>
        </View>
      )}

      {/* Conversations List */}
      {isLoading ? (
        <View className="flex-1 items-center justify-center">
          <ActivityIndicator size="large" color={colors.primary[500]} />
        </View>
      ) : (
        <FlashList
          data={listRows}
          renderItem={renderRow}
          keyExtractor={keyExtractor}
          contentContainerStyle={{ paddingBottom: 80 }}
          showsVerticalScrollIndicator={false}
          ListEmptyComponent={
            <View className="flex-1 items-center justify-center pt-16">
              {/* Icon with subtle glow */}
              <View
                className="w-20 h-20 rounded-full items-center justify-center mb-4"
                style={{
                  backgroundColor: 'rgba(139, 92, 246, 0.1)',
                  shadowColor: colors.pierre.violet,
                  shadowOffset: { width: 0, height: 0 },
                  shadowOpacity: 0.3,
                  shadowRadius: 20,
                }}
              >
                <Feather
                  name={searchQuery ? 'search' : 'message-circle'}
                  size={36}
                  color={colors.pierre.violet}
                />
              </View>
              <Text className="text-lg font-semibold text-text-primary mb-1">
                {searchQuery ? 'No Results' : 'No Conversations'}
              </Text>
              <Text className="text-base text-text-secondary text-center px-6">
                {searchQuery ? 'Try a different search term' : 'Start a conversation with your AI coach'}
              </Text>
            </View>
          }
        />
      )}

      {/* Floating Bottom Bar with Search and New Chat */}
      <View
        className="absolute left-3 right-3 flex-row items-center gap-3"
        style={{ bottom: spacing.lg }}
      >
        <View
          className="flex-1 flex-row items-center px-4 py-2"
          style={[{ height: 48 }, searchBarStyle]}
        >
          <Feather name="search" size={18} color={colors.text.tertiary} />
          <TextInput
            className="flex-1 text-base text-text-primary ml-3"
            placeholder="Search conversations"
            placeholderTextColor={colors.text.tertiary}
            value={searchQuery}
            onChangeText={setSearchQuery}
          />
        </View>
        <TouchableOpacity
          className="w-12 h-12 rounded-full items-center justify-center"
          style={fabStyle}
          onPress={handleNewChat}
        >
          <Feather name="plus" size={24} color="#FFFFFF" />
        </TouchableOpacity>
      </View>

      {/* Action Menu Modal */}
      <Modal
        visible={actionMenuVisible}
        animationType="fade"
        transparent
        onRequestClose={closeActionMenu}
      >
        <TouchableOpacity
          className="flex-1 bg-black/50 justify-center items-center"
          activeOpacity={1}
          onPress={closeActionMenu}
        >
          <View
            className="min-w-[240px] overflow-hidden"
            style={menuStyle}
          >
            {/* Gradient accent bar */}
            <LinearGradient
              colors={gradients.violetCyan as [string, string]}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ height: 3, width: '100%' }}
            />
            <View className="py-2">
              <TouchableOpacity
                className="flex-row items-center px-4 py-3 opacity-40"
                disabled
              >
                <Feather name="star" size={18} color={colors.text.tertiary} />
                <Text className="text-base text-text-tertiary ml-3">Add to favorites</Text>
              </TouchableOpacity>

              <TouchableOpacity className="flex-row items-center px-4 py-3" onPress={handleRename}>
                <Feather name="edit-2" size={18} color={colors.text.primary} />
                <Text className="text-base text-text-primary ml-3">Rename</Text>
              </TouchableOpacity>

              <TouchableOpacity className="flex-row items-center px-4 py-3" onPress={handleDelete}>
                <Feather name="trash-2" size={18} color={colors.error} />
                <Text className="text-base text-error ml-3">Delete</Text>
              </TouchableOpacity>
            </View>
          </View>
        </TouchableOpacity>
      </Modal>

      {/* Rename Conversation Prompt Dialog */}
      <PromptDialog
        visible={renamePromptVisible}
        title="Rename Conversation"
        message="Enter a new name for this conversation"
        defaultValue={selectedConversation?.title || ''}
        submitText="Save"
        cancelText="Cancel"
        onSubmit={handleRenameSubmit}
        onCancel={handleRenameCancel}
        testID="rename-conversation-dialog"
      />
    </SafeAreaView>
  );
}
