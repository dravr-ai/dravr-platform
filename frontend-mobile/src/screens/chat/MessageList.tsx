// ABOUTME: Message list component with FlatList rendering and empty states
// ABOUTME: Handles message display, thinking indicator, and coach grid for new chats

import React, { useState, useMemo } from 'react';
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
import { PRIMARY_PALETTE, PROVIDER_COLORS, spacing, fontSize, borderRadius, aiGlow, useThemeColors, useTheme } from '../../constants/theme';
import type { Message, Coach } from '../../types';

type ThemeColors = ReturnType<typeof useThemeColors>;

// Coach category emoji icons
const COACH_CATEGORY_ICONS: Record<string, string> = {
  training: '🏃',
  nutrition: '🥗',
  recovery: '😴',
  recipes: '👨‍🍳',
  mobility: '🧘',
  custom: '⚙️',
};

// Markdown styles for assistant messages — built per palette so the rendered
// markdown flips with the active theme.
const buildMarkdownStyles = (colors: ThemeColors) => ({
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
    color: PRIMARY_PALETTE[400],
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
    color: PRIMARY_PALETTE[400],
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
});

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
  const colors = useThemeColors();
  const markdownStyles = useMemo(() => buildMarkdownStyles(colors), [colors]);
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

export interface MessageActionButton {
  label: string;
  action_type: string;
  value: string;
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
  /** Slash-command action buttons keyed by assistant message id. */
  messageActions?: Record<string, MessageActionButton[]>;
  flatListRef: React.RefObject<FlashListRef<Message> | null>;
  onScrollToBottom: () => void;
  onCoachSelect: (coach: Coach) => void;
  onCreateInsight: (content: string) => void;
  onShareToFeed: (content: string) => void;
  onThumbsUp: (messageId: string) => void;
  onThumbsDown: (messageId: string) => void;
  onRetryMessage: (messageId: string) => void;
  onOpenUrl: (url: string) => void;
  /** Click handler for a slash-command action button. */
  onActionClick?: (action: MessageActionButton) => void;
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
  messageActions,
  flatListRef,
  onScrollToBottom,
  onCoachSelect,
  onCreateInsight,
  onShareToFeed,
  onThumbsUp,
  onThumbsDown,
  onRetryMessage,
  onOpenUrl,
  onActionClick,
}: MessageListProps) {
  const colors = useThemeColors();
  const markdownStyles = useMemo(() => buildMarkdownStyles(colors), [colors]);
  const { scheme } = useTheme();
  const isDark = scheme === 'dark';
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
                style={{ backgroundColor: PROVIDER_COLORS.strava }}
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
          /* User message — right-aligned bubble. Uses surface-container-high
             so it sits one tier above the canvas in both modes (clearly
             darker than cream in light, clearly lighter than ink in dark)
             plus a strong hairline border for added separation. */
          <View
            className="max-w-[85%] rounded-2xl rounded-br-[4px] px-4 py-3"
            style={{
              backgroundColor: colors.tokens.surfaceContainerHigh,
              borderWidth: 1,
              borderColor: colors.border.strong,
            }}
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
        {/* Slash-command action buttons (e.g. per-coach select on /coach).
            Rendered below the body; tap posts the button's value as the
            next message through the same dispatch pipeline. Not
            persisted — buttons only appear on the turn that produced
            them. */}
        {!isUser && messageActions && messageActions[item.id] && messageActions[item.id].length > 0 && (
          <View className="flex-row flex-wrap mt-3 gap-2">
            {messageActions[item.id].map((action, idx) => (
              <TouchableOpacity
                key={`${action.value}-${idx}`}
                className="px-3 py-2 rounded-lg bg-pierre-violet/15"
                onPress={() => onActionClick?.(action)}
              >
                <Text className="text-sm text-pierre-violet font-medium">{action.label}</Text>
              </TouchableOpacity>
            ))}
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
          backgroundColor: colors.background.elevated,
          borderWidth: 1,
          borderColor: colors.border.strong,
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
    // Pillar tint per category — drives the icon container, the small caps
    // label, and the favorite star so a card reads as one cohesive object.
    const pillarKey = (
      ['training', 'nutrition', 'recovery', 'mobility', 'recipes'] as const
    ).includes(coach.category as never)
      ? (coach.category === 'recipes' ? 'nutrition' : (coach.category as 'training' | 'nutrition' | 'recovery' | 'mobility'))
      : null;
    const pillarMap = {
      training: colors.pierre.activity,
      nutrition: colors.pierre.nutrition,
      recovery: colors.pierre.recovery,
      mobility: colors.pierre.mobility,
    } as const;
    const pillarColor = pillarKey ? pillarMap[pillarKey] : colors.pierre.violet;

    // Light surfaces use a hairline outline-variant edge + soft 4% ink
    // shadow; dark surfaces lean on a deeper black drop so cards float over
    // the near-black canvas. Both sit on the cream/ink base canvas one tier up.
    const cardBg = colors.background.elevated;
    const cardBorder = isDark
      ? 'rgba(192, 200, 195, 0.10)'
      : 'rgba(26, 28, 27, 0.06)';
    const cardShadow = {
      shadowColor: isDark ? '#000000' : '#1a1c1b',
      shadowOffset: { width: 0, height: 4 },
      shadowOpacity: isDark ? 0.35 : 0.04,
      shadowRadius: 16,
      elevation: 3,
    };

    return (
      <TouchableOpacity
        key={coach.id}
        className="rounded-2xl px-5 pt-4 pb-5 mb-3"
        style={{
          backgroundColor: cardBg,
          borderWidth: 1,
          borderColor: cardBorder,
          ...cardShadow,
        }}
        onPress={() => onCoachSelect(coach)}
        activeOpacity={0.85}
      >
        {/* Top row: pillar-tinted icon tile + category label + chevron */}
        <View className="flex-row items-center mb-3">
          <View
            className="w-9 h-9 rounded-xl items-center justify-center mr-3"
            style={{ backgroundColor: `${pillarColor}1F` }}
          >
            <Text className="text-base">{COACH_CATEGORY_ICONS[coach.category]}</Text>
          </View>
          <Text
            className="text-[11px] font-semibold uppercase tracking-[0.12em] flex-1"
            style={{ color: pillarColor }}
          >
            {coach.category}
          </Text>
          {coach.is_favorite && (
            <Text className="text-sm mr-1" style={{ color: colors.pierre.nutrition }}>★</Text>
          )}
          <Text className="text-lg" style={{ color: colors.text.tertiary }}>›</Text>
        </View>

        {/* Coach name — large editorial weight */}
        <Text
          className="text-lg font-bold mb-1"
          style={{ color: colors.text.primary }}
          numberOfLines={2}
        >
          {coach.title}
        </Text>

        {/* Description */}
        {coach.description && (
          <Text
            className="text-sm leading-5"
            style={{ color: colors.text.secondary }}
            numberOfLines={5}
          >
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
        <ActivityIndicator size="large" color={PRIMARY_PALETTE[500]} />
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
