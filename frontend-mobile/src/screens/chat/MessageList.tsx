// ABOUTME: Chat message list — one switch over the turn's reply blocks, plus the coach grid and empty states
// ABOUTME: The server decided what this surface draws; nothing here re-derives it from the reply prose

import React, { useEffect, useState, useMemo } from 'react';
import {
  View,
  Text,
  TextInput,
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
import { countActivities, isToolPlumbingMessage, transcriptBlocks } from '@pierre/chat-utils';
import { PRIMARY_PALETTE, spacing, fontSize, borderRadius, aiGlow, useThemeColors, useTheme } from '../../constants/theme';
import type { Message, Coach } from '../../types';
import type { ChatMessageAction, ClaimVerdict, ReplyBlock, VerdictTone } from '@pierre/shared-types';
import {
  mergeVerdictSeverities,
  parseWorkoutPlan,
  summarizeVerdicts,
  verdictSummaryLabel,
} from '@pierre/shared-types';
import type { RenderBlock } from '@pierre/scene-types';
import { parseSceneBlocks, splitVizMarkers } from '@pierre/chat-utils';
import SceneView from './SceneView';
import WorkoutPlanCard from './WorkoutPlanCard';
import { MARKDOWN_RULES, TABLE_CELL_MIN_WIDTH } from './markdownRules';

type ThemeColors = ReturnType<typeof useThemeColors>;

/**
 * Optional "what went wrong?" reason captured after a thumbs-down. The down
 * rating is already persisted; submitting only adds/updates the comment on the
 * same feedback row. Pre-fills with any saved reason on reload.
 */
function FeedbackReasonInput({
  initialComment,
  onSubmit,
  colors,
}: {
  initialComment?: string;
  onSubmit: (comment: string) => void;
  colors: ThemeColors;
}) {
  const [value, setValue] = useState(initialComment ?? '');
  const [saved, setSaved] = useState(false);

  // The saved reason can arrive after mount (conversation reload).
  useEffect(() => {
    setValue(initialComment ?? '');
  }, [initialComment]);

  const submit = () => {
    onSubmit(value.trim());
    setSaved(true);
  };

  return (
    <View className="flex-row items-center gap-2 mt-2 ml-1">
      <TextInput
        value={value}
        onChangeText={(text) => {
          setValue(text);
          setSaved(false);
        }}
        onSubmitEditing={submit}
        placeholder="What went wrong? (optional)"
        placeholderTextColor={colors.text.tertiary}
        returnKeyType="done"
        className="flex-1 px-3 py-1.5 rounded-lg text-xs"
        style={{
          color: colors.text.primary,
          backgroundColor: colors.background.elevated,
          borderWidth: 1,
          borderColor: colors.border.strong,
        }}
      />
      <TouchableOpacity
        className="px-3 py-1.5 rounded-lg"
        style={{ backgroundColor: colors.background.elevated }}
        onPress={submit}
      >
        <Text className="text-xs" style={{ color: colors.pierre.violet }}>
          {saved ? 'Saved' : 'Send'}
        </Text>
      </TouchableOpacity>
    </View>
  );
}

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
  // Cells size to their content with a floor, rather than shrinking to fit the
  // screen. Squeezing a 5-column table into a phone width wraps every heading
  // to one letter per line; the horizontal ScrollView in MARKDOWN_RULES.table
  // is what makes the overflow reachable instead of clipped.
  th: {
    padding: 8,
    minWidth: TABLE_CELL_MIN_WIDTH,
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
    minWidth: TABLE_CELL_MIN_WIDTH,
    borderRightWidth: 1,
    borderColor: colors.border.subtle,
    fontSize: fontSize.sm,
    color: colors.text.secondary,
  },
});

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
          <Markdown style={markdownStyles} rules={MARKDOWN_RULES}>{activityText}</Markdown>
        </View>
      )}
    </View>
  );
}

/** Feedback tint for the tone the shared rollup assigned the worst verdict. */
function verdictChipColor(tone: VerdictTone, colors: ThemeColors): string {
  switch (tone) {
    case 'success':
      return colors.success;
    case 'warning':
      return colors.warning;
    case 'error':
      return colors.error;
    case 'info':
      return colors.info;
    case 'secondary':
    default:
      return colors.text.secondary;
  }
}

interface MessageListProps {
  messages: Message[];
  coaches: Coach[];
  isLoading: boolean;
  isSending: boolean;
  isCoachConversation: boolean;
  messageFeedback: Record<string, 'up' | 'down' | null>;
  /** Saved thumbs-down reasons, keyed by message id. */
  messageFeedbackComment: Record<string, string>;
  insightMessages: Set<string>;
  /**
   * What the server decided this surface draws, keyed by assistant message id.
   *
   * Present on the turn that produced it. A message with no entry is drawn
   * from its transcript row, decoded into the same block shape — so the switch
   * below walks exactly one list either way.
   */
  messageBlocks?: Record<string, ReplyBlock[]>;
  /** Claim verdicts for the active conversation, keyed by message_id. */
  verdicts?: ClaimVerdict[];
  flatListRef: React.RefObject<FlashListRef<Message> | null>;
  onScrollToBottom: () => void;
  onCoachSelect: (coach: Coach) => void;
  onCreateInsight: (content: string) => void;
  onShareToFeed: (content: string) => void;
  onThumbsUp: (messageId: string) => void;
  onThumbsDown: (messageId: string) => void;
  /** Persist an optional thumbs-down reason for a message. */
  onSubmitFeedbackReason: (messageId: string, comment: string) => void;
  onRetryMessage: (messageId: string) => void;
  onOpenUrl: (url: string) => void;
  /** Press handler for a control the reply's `actions` block carried. */
  onActionClick?: (action: ChatMessageAction) => void;
}

export function MessageList({
  messages,
  coaches,
  isLoading,
  isSending,
  isCoachConversation,
  messageFeedback,
  messageFeedbackComment,
  insightMessages,
  messageBlocks,
  verdicts,
  flatListRef,
  onScrollToBottom,
  onCoachSelect,
  onCreateInsight,
  onShareToFeed,
  onThumbsUp,
  onThumbsDown,
  onSubmitFeedbackReason,
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

  /** The reply's own markdown, with its charts drawn where its markers sit. */
  const renderProse = (text: string, scenes: RenderBlock[], key: number) => (
    <View key={key}>
      {splitVizMarkers(text).map((segment, i) =>
        segment.kind === 'prose' ? (
          <Markdown
            key={i}
            style={markdownStyles}
            rules={MARKDOWN_RULES}
            onLinkPress={(url) => { onOpenUrl(url); return false; }}
          >
            {segment.text}
          </Markdown>
        ) : scenes[segment.index] ? (
          <SceneView key={i} block={scenes[segment.index]} />
        ) : null,
      )}
    </View>
  );

  /**
   * Draw one reply block.
   *
   * The only place this surface decides what a piece of a reply looks like.
   * Every arm renders what the server already chose to send; none of them
   * reads the prose to work out whether a different arm should have fired.
   */
  const renderBlock = (
    block: ReplyBlock,
    key: number,
    context: { isUser: boolean; scenes: RenderBlock[]; rows: ClaimVerdict[] },
  ): React.ReactNode => {
    switch (block.type) {
      case 'prose':
        return context.isUser ? (
          <Text key={key} className="text-base text-text-primary leading-6">
            {block.text}
          </Text>
        ) : (
          renderProse(block.text, context.scenes, key)
        );

      case 'activity_list':
        return <CollapsibleActivities key={key} activityText={block.text} />;

      case 'workout_plan': {
        const plan = parseWorkoutPlan(JSON.stringify(block.plan));
        return plan ? <WorkoutPlanCard key={key} plan={plan} /> : null;
      }

      // The charts this block carries are positioned by the prose's own
      // ⟦viz:N⟧ markers and drawn in the prose arm. Drawing them here too is
      // the same chart twice.
      case 'scene':
        return null;

      case 'scene_image':
        return (
          <Image
            key={key}
            source={{ uri: block.url }}
            className="w-full h-48 rounded-xl mt-3"
            resizeMode="contain"
            accessibilityLabel={block.caption ?? 'Chart'}
          />
        );

      case 'verdicts': {
        const summary = summarizeVerdicts(mergeVerdictSeverities(context.rows, block.chips));
        if (!summary) return null;
        const tint = verdictChipColor(summary.tone, colors);
        return (
          <View
            key={key}
            testID="verdict-chip"
            accessibilityLabel={`Claim verdicts: ${summary.worstStatus}`}
            className="flex-row items-center self-start mt-2 px-2 py-1 rounded-full"
            style={{ backgroundColor: `${tint}26` }}
          >
            <Ionicons name="shield-half-outline" size={12} color={tint} />
            <Text className="text-xs ml-1" style={{ color: tint }}>
              {verdictSummaryLabel(summary)}
            </Text>
          </View>
        );
      }

      case 'actions':
        return (
          <View key={key} className="mt-3">
            {block.title ? (
              <Text className="text-xs font-medium text-text-secondary mb-1.5">{block.title}</Text>
            ) : null}
            <View className="flex-row flex-wrap gap-2">
              {block.actions.map((action, idx) => (
                <TouchableOpacity
                  key={`${action.value}-${idx}`}
                  className="px-3 py-2 rounded-lg bg-pierre-violet/15"
                  onPress={() => onActionClick?.(action)}
                >
                  <Text className="text-sm text-primary font-medium">{action.label}</Text>
                </TouchableOpacity>
              ))}
            </View>
          </View>
        );

      case 'reconnect':
        return (
          <TouchableOpacity
            key={key}
            className="mt-3 self-start px-4 py-2 rounded-lg bg-primary"
            onPress={() => onOpenUrl(block.url)}
          >
            <Text className="text-sm font-medium text-on-primary">
              Reconnect {block.display_name}
            </Text>
          </TouchableOpacity>
        );

      // A quota notice is a fact about the turn, not about the message: the
      // usage banner below the transcript shows it once.
      case 'notice':
        return null;
    }
  };

  const renderMessage = ({ item }: { item: Message }) => {
    if (!item?.id) return null;
    // Defensive: never render internal tool plumbing (tool_call / tool_result).
    // The list is pre-filtered in useMessages, but a stray row from any other
    // path must not dump raw <tool_call>/<tool_result> XML at the user.
    if (isToolPlumbingMessage(item)) return null;

    const isUser = item.role === 'user';
    const isError = item.isError === true;
    const rows = (verdicts ?? []).filter((verdict) => verdict.message_id === item.id);
    // One list, whatever the turn's age: the server's own blocks when it just
    // landed, the same shape decoded from the persisted row when it did not.
    const blocks = messageBlocks?.[item.id] ?? transcriptBlocks(item, rows);
    const sceneBlock = blocks.find((block) => block.type === 'scene');
    const scenes = sceneBlock?.type === 'scene' ? parseSceneBlocks(sceneBlock.scene_blocks) : [];
    const context = { isUser, scenes, rows };

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
            {blocks.map((block, index) => renderBlock(block, index, context))}
          </View>
        ) : (
          /* Assistant message — full-width, no bubble, like Claude */
          <View
            className={`w-full ${isError ? 'bg-error/10 rounded-xl p-3 border border-error/30' : ''}`}
          >
            {blocks.map((block, index) => renderBlock(block, index, context))}
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
        {/* Optional thumbs-down reason — the down rating is already saved; this
            adds/updates the free-text comment on the same feedback row. */}
        {!isUser && !isError && messageFeedback[item.id] === 'down' && (
          <FeedbackReasonInput
            initialComment={messageFeedbackComment[item.id]}
            onSubmit={(comment) => onSubmitFeedbackReason(item.id, comment)}
            colors={colors}
          />
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
          <View className="w-2 h-2 rounded-full bg-primary opacity-60" />
          <View className="w-2 h-2 rounded-full bg-primary opacity-80" />
          <View className="w-2 h-2 rounded-full bg-primary" />
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
