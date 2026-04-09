// ABOUTME: Message list component with FlatList rendering and empty states
// ABOUTME: Handles message display, thinking indicator, and coach grid for new chats

import React, { useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ActivityIndicator,
  ScrollView,
  Platform,
  Image,
} from 'react-native';
import { FlashList, type FlashListRef } from '@shopify/flash-list';
import Markdown from 'react-native-markdown-display';
import { Ionicons } from '@expo/vector-icons';
import * as Clipboard from 'expo-clipboard';
import { Alert, Share } from 'react-native';
import { splitActivityContent, countActivities } from '@pierre/chat-utils';
import { colors, spacing, fontSize, borderRadius, aiGlow } from '../../constants/theme';
import type { Message, Coach } from '../../types';

// Coach category badge background colors
const COACH_CATEGORY_BADGE_BG: Record<string, string> = {
  training: 'rgba(16, 185, 129, 0.15)',
  nutrition: 'rgba(245, 158, 11, 0.15)',
  recovery: 'rgba(99, 102, 241, 0.15)',
  recipes: 'rgba(249, 115, 22, 0.15)',
  mobility: 'rgba(236, 72, 153, 0.15)',
  custom: 'rgba(124, 58, 237, 0.15)',
};

// Coach category emoji icons
const COACH_CATEGORY_ICONS: Record<string, string> = {
  training: '🏃',
  nutrition: '🥗',
  recovery: '😴',
  recipes: '👨‍🍳',
  mobility: '🧘',
  custom: '⚙️',
};

// Markdown styles for assistant messages
const markdownStyles = {
  body: {
    color: colors.text.primary,
    fontSize: fontSize.md,
    lineHeight: fontSize.md * 1.5,
  },
  heading1: {
    color: colors.text.primary,
    fontSize: fontSize.xl,
    fontWeight: '700' as const,
    marginTop: spacing.md,
    marginBottom: spacing.sm,
  },
  heading2: {
    color: colors.text.primary,
    fontSize: fontSize.lg,
    fontWeight: '600' as const,
    marginTop: spacing.sm,
    marginBottom: spacing.xs,
  },
  heading3: {
    color: colors.text.primary,
    fontSize: fontSize.md,
    fontWeight: '600' as const,
    marginTop: spacing.xs,
    marginBottom: spacing.xs,
  },
  strong: {
    color: colors.text.primary,
    fontWeight: '700' as const,
  },
  em: {
    color: colors.text.secondary,
    fontStyle: 'italic' as const,
  },
  bullet_list: {
    marginLeft: spacing.sm,
  },
  ordered_list: {
    marginLeft: spacing.sm,
  },
  list_item: {
    marginBottom: spacing.xs,
  },
  code_inline: {
    backgroundColor: colors.background.tertiary,
    color: colors.primary[400],
    paddingHorizontal: 4,
    borderRadius: 4,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    fontSize: fontSize.sm,
  },
  fence: {
    backgroundColor: colors.background.tertiary,
    borderRadius: borderRadius.sm,
    padding: spacing.sm,
    marginVertical: spacing.xs,
  },
  code_block: {
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    fontSize: fontSize.sm,
    color: colors.text.primary,
  },
  link: {
    color: colors.primary[400],
    textDecorationLine: 'underline' as const,
  },
  hr: {
    backgroundColor: colors.border.default,
    height: 1,
    marginVertical: spacing.sm,
  },
  table: {
    borderWidth: 1,
    borderColor: colors.border.default,
    borderRadius: borderRadius.sm,
    marginVertical: spacing.sm,
  },
  thead: {
    backgroundColor: colors.background.tertiary,
  },
  th: {
    padding: 8,
    borderRightWidth: 1,
    borderBottomWidth: 1,
    borderColor: colors.border.default,
    fontWeight: '600' as const,
    fontSize: fontSize.sm,
    color: colors.text.primary,
  },
  tr: {
    borderBottomWidth: 1,
    borderColor: colors.border.subtle,
    flexDirection: 'row' as const,
  },
  td: {
    padding: 8,
    borderRightWidth: 1,
    borderColor: colors.border.subtle,
    fontSize: fontSize.sm,
    color: colors.text.secondary,
    flexShrink: 1,
  },
};

// Helper to detect OAuth authorization URLs
const isOAuthUrl = (url: string): { isOAuth: boolean; provider: string | null } => {
  try {
    const parsedUrl = new URL(url);
    const hostname = parsedUrl.hostname.toLowerCase();

    if (hostname === 'www.strava.com' || hostname === 'strava.com') {
      if (parsedUrl.pathname.includes('/oauth/authorize')) {
        return { isOAuth: true, provider: 'Strava' };
      }
    }
    if (hostname === 'www.fitbit.com' || hostname === 'fitbit.com') {
      if (parsedUrl.pathname.includes('/oauth2/authorize')) {
        return { isOAuth: true, provider: 'Fitbit' };
      }
    }
    if (hostname.endsWith('.garmin.com') || hostname === 'garmin.com') {
      if (url.includes('oauth')) {
        return { isOAuth: true, provider: 'Garmin' };
      }
    }
    return { isOAuth: false, provider: null };
  } catch {
    return { isOAuth: false, provider: null };
  }
};

// Collapsible section for the activity list — closed by default
function CollapsibleActivities({ activityText }: { activityText: string }) {
  const [expanded, setExpanded] = useState(false);

  const activityCount = countActivities(activityText);
  const label = `Your Activities (${activityCount})`;

  return (
    <View className="mb-2">
      <TouchableOpacity
        className="flex-row items-center py-2"
        onPress={() => setExpanded((prev) => !prev)}
        activeOpacity={0.7}
      >
        <Ionicons
          name={expanded ? 'chevron-down' : 'chevron-forward'}
          size={16}
          color={colors.text.secondary}
        />
        <Text className="text-sm font-medium text-text-secondary ml-1">
          {label}
        </Text>
      </TouchableOpacity>
      {expanded && (
        <View className="ml-4">
          <Markdown style={markdownStyles}>{activityText}</Markdown>
        </View>
      )}
    </View>
  );
}

interface MessageListProps {
  messages: Message[];
  coaches: Coach[];
  isLoading: boolean;
  isSending: boolean;
  isCoachConversation: boolean;
  messageFeedback: Record<string, 'up' | 'down' | null>;
  insightMessages: Set<string>;
  /** Activity lists keyed by assistant message ID (from new API field) */
  activityLists: Record<string, string>;
  flatListRef: React.RefObject<FlashListRef<Message> | null>;
  onScrollToBottom: () => void;
  onCoachSelect: (coach: Coach) => void;
  onCreateInsight: (content: string) => void;
  onShareToFeed: (content: string) => void;
  onThumbsUp: (messageId: string) => void;
  onThumbsDown: (messageId: string) => void;
  onRetryMessage: (messageId: string) => void;
  onOpenUrl: (url: string) => void;
}

export function MessageList({
  messages,
  coaches,
  isLoading,
  isSending,
  isCoachConversation,
  messageFeedback,
  insightMessages,
  activityLists,
  flatListRef,
  onScrollToBottom,
  onCoachSelect,
  onCreateInsight,
  onShareToFeed,
  onThumbsUp,
  onThumbsDown,
  onRetryMessage,
  onOpenUrl,
}: MessageListProps) {
  const handleCopyMessage = async (content: string) => {
    try {
      await Clipboard.setStringAsync(content);
      Alert.alert('Copied', 'Message copied to clipboard');
    } catch (error) {
      console.error('Failed to copy:', error);
    }
  };

  const handleShareMessage = async (content: string) => {
    try {
      await Share.share({ message: content });
    } catch (error) {
      console.error('Failed to share:', error);
    }
  };

  const renderMessageContent = (content: string, isUser: boolean, messageId?: string) => {
    if (isUser) {
      return (
        <Text className="text-base text-text-primary leading-6">
          {content}
        </Text>
      );
    }

    const urlRegex = /https?:\/\/[^\s<>"\]]+/gi;
    const oauthUrls = content.match(urlRegex)?.filter(url => {
      const { isOAuth } = isOAuthUrl(url);
      return isOAuth;
    }) || [];

    if (oauthUrls.length > 0) {
      let cleanContent = content;
      oauthUrls.forEach(url => {
        const escapedUrl = url.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
        cleanContent = cleanContent.replace(new RegExp(`!\\[([^\\]]*)\\]\\(${escapedUrl}\\)`, 'g'), '');
        cleanContent = cleanContent.replace(new RegExp(`\\[([^\\]]*)\\]\\(${escapedUrl}\\)`, 'g'), '');
        cleanContent = cleanContent.replace(url, '');
      });

      return (
        <View className="flex-row flex-wrap items-center">
          {oauthUrls.map((url, index) => {
            const { provider } = isOAuthUrl(url);
            return (
              <TouchableOpacity
                key={`oauth-${index}`}
                className="px-4 py-2 rounded-lg my-1 self-start"
                style={{ backgroundColor: colors.providers.strava }}
                onPress={() => onOpenUrl(url)}
              >
                <Text className="text-base font-semibold text-text-primary">
                  Connect to {provider}
                </Text>
              </TouchableOpacity>
            );
          })}
          {cleanContent.trim() && (
            <Markdown style={markdownStyles} onLinkPress={(url) => { onOpenUrl(url); return false; }}>
              {cleanContent.trim()}
            </Markdown>
          )}
        </View>
      );
    }

    // Check for activity list: new API field first, then fall back to parsing old content
    const apiActivityList = messageId ? activityLists[messageId] : undefined;
    const [parsedActivityList, analysisContent] = apiActivityList
      ? [null, content]
      : splitActivityContent(content);
    const activityText = apiActivityList ?? parsedActivityList;

    if (activityText) {
      return (
        <View>
          <CollapsibleActivities activityText={activityText} />
          <Markdown style={markdownStyles} onLinkPress={(url) => { onOpenUrl(url); return false; }}>
            {apiActivityList ? content : analysisContent}
          </Markdown>
        </View>
      );
    }

    return (
      <Markdown style={markdownStyles} onLinkPress={(url) => { onOpenUrl(url); return false; }}>
        {content}
      </Markdown>
    );
  };

  const renderMessage = ({ item }: { item: Message }) => {
    if (!item?.id) return null;

    const isUser = item.role === 'user';
    const isError = item.isError === true;

    return (
      <View className={`mb-4 ${isUser ? 'items-end' : ''}`}>
        {isUser ? (
          /* User message — right-aligned bubble with distinct background */
          <View
            className="max-w-[85%] rounded-2xl rounded-br-[4px] px-4 py-3"
            style={{ backgroundColor: 'rgba(139, 92, 246, 0.15)', borderWidth: 1, borderColor: 'rgba(139, 92, 246, 0.25)' }}
          >
            {renderMessageContent(item.content, true, item.id)}
          </View>
        ) : (
          /* Assistant message — full-width, no bubble, like Claude */
          <View
            className={`w-full ${isError ? 'bg-error/10 rounded-xl p-3 border border-error/30' : ''}`}
          >
            {renderMessageContent(item.content, false, item.id)}
          </View>
        )}
        {!isUser && (
          <View className="flex-row mt-1 gap-4">
            {isError ? (
              <TouchableOpacity
                className="flex-row items-center bg-background-tertiary px-2 py-1 rounded gap-1"
                onPress={() => onRetryMessage(item.id)}
              >
                <Ionicons name="refresh-outline" size={14} color={colors.text.primary} />
                <Text className="text-xs text-text-primary font-medium">Retry</Text>
              </TouchableOpacity>
            ) : (
              <>
                <TouchableOpacity className="p-0.5" onPress={() => handleCopyMessage(item.content)}>
                  <Ionicons name="copy-outline" size={14} color={colors.text.tertiary} />
                </TouchableOpacity>
                <TouchableOpacity className="p-0.5" onPress={() => handleShareMessage(item.content)}>
                  <Ionicons name="arrow-redo-outline" size={14} color={colors.text.tertiary} />
                </TouchableOpacity>
                {!insightMessages.has(item.id) && (
                  <TouchableOpacity className="p-0.5" onPress={() => onCreateInsight(item.content)}>
                    <Ionicons name="bulb-outline" size={14} color={colors.text.tertiary} />
                  </TouchableOpacity>
                )}
                {insightMessages.has(item.id) && (
                  <TouchableOpacity className="p-0.5" onPress={() => onShareToFeed(item.content)}>
                    <Ionicons name="people-outline" size={14} color={colors.text.tertiary} />
                  </TouchableOpacity>
                )}
                <TouchableOpacity className="p-0.5" onPress={() => onThumbsUp(item.id)}>
                  <Ionicons
                    name={messageFeedback[item.id] === 'up' ? 'thumbs-up' : 'thumbs-up-outline'}
                    size={14}
                    color={messageFeedback[item.id] === 'up' ? colors.pierre.violet : colors.text.tertiary}
                  />
                </TouchableOpacity>
                <TouchableOpacity className="p-0.5" onPress={() => onThumbsDown(item.id)}>
                  <Ionicons
                    name={messageFeedback[item.id] === 'down' ? 'thumbs-down' : 'thumbs-down-outline'}
                    size={14}
                    color={messageFeedback[item.id] === 'down' ? colors.error : colors.text.tertiary}
                  />
                </TouchableOpacity>
                <TouchableOpacity className="p-0.5" onPress={() => onRetryMessage(item.id)}>
                  <Ionicons name="refresh-outline" size={14} color={colors.text.tertiary} />
                </TouchableOpacity>
                {item.model && (
                  <Text className="text-xs text-text-tertiary ml-2">
                    {item.model}{item.execution_time_ms ? ` · ${(item.execution_time_ms / 1000).toFixed(1)}s` : ''}
                  </Text>
                )}
              </>
            )}
          </View>
        )}
      </View>
    );
  };

  const renderThinkingIndicator = () => (
    <View className="mb-4" testID="thinking-indicator">
      <View
        className="flex-row max-w-[85%] rounded-2xl rounded-bl-[4px] p-4"
        style={{
          backgroundColor: 'rgba(30, 30, 46, 0.9)',
          borderWidth: 1,
          borderColor: 'rgba(139, 92, 246, 0.3)',
          ...aiGlow.thinking,
        }}
      >
        <View className="w-8 h-8 rounded-full mr-3 overflow-hidden">
          <Image
            source={require('../../../assets/dravr-logo.png')}
            className="w-8 h-8"
            resizeMode="cover"
          />
        </View>
        <View className="flex-row items-center gap-1">
          <View className="w-2 h-2 rounded-full bg-pierre-violet opacity-60" />
          <View className="w-2 h-2 rounded-full bg-pierre-violet opacity-80" />
          <View className="w-2 h-2 rounded-full bg-pierre-violet" />
        </View>
      </View>
    </View>
  );

  const renderCoachCard = (coach: Coach) => {
    const categoryColor = COACH_CATEGORY_BADGE_BG[coach.category] || 'rgba(124, 58, 237, 0.15)';

    return (
      <TouchableOpacity
        key={coach.id}
        className="bg-background-secondary rounded-2xl px-5 pt-4 pb-5 mb-3"
        onPress={() => onCoachSelect(coach)}
        activeOpacity={0.7}
      >
        {/* Top row: category icon + category label + chevron */}
        <View className="flex-row items-center mb-3">
          <View
            className="w-8 h-8 rounded-lg items-center justify-center mr-2"
            style={{ backgroundColor: categoryColor }}
          >
            <Text className="text-base">{COACH_CATEGORY_ICONS[coach.category]}</Text>
          </View>
          <Text className="text-xs font-semibold uppercase tracking-wide flex-1" style={{ color: colors.pierre.violet }}>
            {coach.category}
          </Text>
          {coach.is_favorite && (
            <Text className="text-sm mr-1" style={{ color: '#F59E0B' }}>★</Text>
          )}
          <Text className="text-lg text-text-tertiary">›</Text>
        </View>

        {/* Coach name — large, like Health app values */}
        <Text className="text-lg font-bold text-text-primary mb-1" numberOfLines={2}>
          {coach.title}
        </Text>

        {/* Description */}
        {coach.description && (
          <Text className="text-sm text-text-secondary leading-5" numberOfLines={5}>
            {coach.description}
          </Text>
        )}
      </TouchableOpacity>
    );
  };

  const renderEmptyChat = () => (
    <ScrollView
      className="flex-1"
      contentContainerStyle={{ flexGrow: 1, alignItems: 'center', justifyContent: 'flex-start', paddingHorizontal: spacing.xs, paddingVertical: spacing.md, paddingBottom: 140 }}
      showsVerticalScrollIndicator={false}
      keyboardShouldPersistTaps="handled"
    >
      {!isCoachConversation && coaches.length > 0 && (
        <View className="w-full px-1">
          <Text className="text-sm font-semibold text-text-tertiary uppercase tracking-wide mb-3 ml-1">Your Coaches</Text>
          {coaches.map((coach) => renderCoachCard(coach))}
        </View>
      )}

      {!isCoachConversation && coaches.length === 0 && (
        <View className="flex-1 items-center justify-center px-8 py-12">
          <Text className="text-lg font-semibold text-text-primary mb-2">No coaches yet</Text>
          <Text className="text-base text-text-tertiary text-center">
            Create your first coach to customize how Dravr helps you.
          </Text>
        </View>
      )}

      {isCoachConversation && (
        <View className="w-full items-center px-4 mb-6">
          <Text className="text-base text-text-secondary text-center leading-6">
            Your coach is ready. Start the conversation by typing a message below.
          </Text>
        </View>
      )}
    </ScrollView>
  );

  if (isLoading) {
    return (
      <View className="flex-1 items-center justify-center">
        <ActivityIndicator size="large" color={colors.primary[500]} />
      </View>
    );
  }

  if ((messages?.length ?? 0) === 0 && !isSending) {
    return renderEmptyChat();
  }

  return (
    <View style={{ flex: 1 }} testID="messages-list">
      <FlashList
        ref={flatListRef}
        data={messages ?? []}
        renderItem={renderMessage}
        keyExtractor={(item, index) => item?.id ?? `fallback-${index}`}

        contentContainerStyle={{ paddingHorizontal: spacing.md, paddingVertical: spacing.md, paddingBottom: 140 }}
        showsVerticalScrollIndicator={false}
        onContentSizeChange={onScrollToBottom}
        ListFooterComponent={isSending ? renderThinkingIndicator : null}
      />
    </View>
  );
}
