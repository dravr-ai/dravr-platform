// ABOUTME: Chat message list — one switch over the turn's reply blocks, and the empty thread's one line
// ABOUTME: The server decided what this surface draws; nothing here re-derives it from the reply prose

import React, { useEffect, useRef, useState, useMemo } from 'react';
import {
  Animated,
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
import {
  copyableText,
  countActivities,
  dayLabelFor,
  filterDisplayMessages,
  formatMessageTime,
  isSameMessageGroup,
  localDayKey,
  transcriptBlocks,
} from '@pierre/chat-utils';
import { linkifyUrls } from '@pierre/domain-utils';
import { SLASH_HINT_KEY, VERDICT_STATUS_LABEL_KEY, verdictChipLabel } from '@pierre/shared-constants';
import { PRIMARY_PALETTE, spacing, fontSize, borderRadius, useThemeColors } from '../../constants/theme';
import type { Message } from '../../types';
import type { ChatMessageAction, ClaimVerdict, ReplyBlock, VerdictTone } from '@pierre/shared-types';
import {
  parseWorkoutPlan,
  summarizeVerdicts,
  verdictChipSeverity,
  verdictToneAlerts,
} from '@pierre/shared-types';
import type { RenderBlock } from '@pierre/scene-types';
import { parseSceneBlocks, splitVizMarkers } from '@pierre/chat-utils';
import DaySeparator from './DaySeparator';
import SceneView from './SceneView';
import WorkoutPlanCard from './WorkoutPlanCard';
import { MARKDOWN_RULES, TABLE_CELL_MIN_WIDTH } from './markdownRules';
import { useTranslation } from '@pierre/i18n';

export type ThemeColors = ReturnType<typeof useThemeColors>;

/**
 * One entry of the rendered thread: a message, or the day pill above the first
 * message of a day.
 *
 * The list draws a projection of the transcript rather than the transcript
 * itself, because a separator is a row of the list like any other — FlashList
 * recycles by `getItemType`, so a pill and a bubble never reuse each other's
 * view. `groupStart` is the same decision the web thread makes: the first row
 * of a run of one author's messages carries the larger gap above it.
 */
export type ChatRow =
  | { kind: 'day'; key: string; label: string }
  | { kind: 'message'; key: string; message: Message; groupStart: boolean };

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
  const { t } = useTranslation();
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
        placeholder={t('app.whatWentWrongOptional')}
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
          {saved ? t('app.blockSaved') : t('app.blockSend')}
        </Text>
      </TouchableOpacity>
    </View>
  );
}

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
function CollapsibleActivities({
  activityText,
  onLinkPress,
}: {
  activityText: string;
  /** Opens a link the activity list carries — an activity's page on the provider. */
  onLinkPress: (url: string) => void;
}) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const markdownStyles = useMemo(() => buildMarkdownStyles(colors), [colors]);
  const [expanded, setExpanded] = useState(false);

  const activityCount = countActivities(activityText);
  const label = t('app.yourActivitiesCount', { count: activityCount });

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
          <Markdown
            style={markdownStyles}
            rules={MARKDOWN_RULES}
            onLinkPress={(url) => { onLinkPress(url); return false; }}
          >
            {activityText}
          </Markdown>
        </View>
      )}
    </View>
  );
}

/** The two halves of a verdict chip: what it is filled with, and what reads on that fill. */
export interface VerdictChipPalette {
  /** The chip's ground — the tone's own hue at container weight. */
  fill: string;
  /** The ink bound to that ground. Label and icon both take it. */
  ink: string;
}

/**
 * Container weight for a feedback tint — 15%, the same fraction the web chip
 * spells as `/15`. Hex alpha, because these fills are composed as strings.
 */
const VERDICT_TINT_ALPHA = '26';

/**
 * Ground and ink for the tone the shared rollup assigns a verdict — the turn's
 * chip, and each card's in the sheet.
 *
 * One function returns both because they are one decision. A hue drawn as text
 * on a tint of itself does not clear AA: light `warning` measures 2.73:1 that
 * way, against 6.41:1 for the ink the token tree binds to that tint. So the
 * fill stays the tone's hue and the label takes `colors.ink.*` — the same hue
 * carried along its lightness axis — or the on-colour the tree already names,
 * where it names one.
 */
export function verdictChipPalette(tone: VerdictTone, colors: ThemeColors): VerdictChipPalette {
  switch (tone) {
    case 'success':
      return { fill: `${colors.success}${VERDICT_TINT_ALPHA}`, ink: colors.ink.success };
    case 'warning':
      return { fill: `${colors.warning}${VERDICT_TINT_ALPHA}`, ink: colors.ink.warning };
    case 'error':
      return { fill: `${colors.error}${VERDICT_TINT_ALPHA}`, ink: colors.tokens.onErrorContainer };
    case 'info':
      return { fill: `${colors.info}${VERDICT_TINT_ALPHA}`, ink: colors.ink.info };
    // A toneless verdict has no hue to carry, so its chip is the neutral pair:
    // a wash of the muted ink under body copy, which is what web draws too.
    case 'secondary':
    default:
      return { fill: `${colors.text.secondary}${VERDICT_TINT_ALPHA}`, ink: colors.text.primary };
  }
}

/** How many dots say "typing". */
const TYPING_DOT_COUNT = 3;
/** The dot's dimmest point — visible, so three dots always read as three. */
const TYPING_DOT_MIN_OPACITY = 0.25;
/** One fade, up or down. */
const TYPING_DOT_FADE_MS = 380;
/** How far each dot trails the one before it, so the row reads as a wave. */
const TYPING_DOT_STAGGER_MS = 180;

/**
 * The endless fade one dot runs.
 *
 * The delay is split before and after the fade so every dot's cycle lasts the
 * same time whatever its position: staggering only the head of the loop would
 * give each dot its own period and the wave would drift apart within seconds.
 */
export function typingDotAnimation(
  opacity: Animated.Value,
  index: number,
): Animated.CompositeAnimation {
  return Animated.loop(
    Animated.sequence([
      Animated.delay(index * TYPING_DOT_STAGGER_MS),
      Animated.timing(opacity, {
        toValue: 1,
        duration: TYPING_DOT_FADE_MS,
        useNativeDriver: false,
      }),
      Animated.timing(opacity, {
        toValue: TYPING_DOT_MIN_OPACITY,
        duration: TYPING_DOT_FADE_MS,
        useNativeDriver: false,
      }),
      Animated.delay((TYPING_DOT_COUNT - 1 - index) * TYPING_DOT_STAGGER_MS),
    ]),
  );
}

/** One breathing dot of the typing indicator. */
function TypingDot({ index }: { index: number }) {
  const opacity = useRef(new Animated.Value(TYPING_DOT_MIN_OPACITY)).current;

  useEffect(() => {
    const animation = typingDotAnimation(opacity, index);
    animation.start();
    return () => animation.stop();
  }, [opacity, index]);

  // The fade rides on the Animated wrapper and the dot's look stays on a plain
  // View, which is the component NativeWind resolves `className` on.
  return (
    <Animated.View testID={`typing-dot-${index}`} style={{ opacity }}>
      <View className="w-2 h-2 rounded-full bg-primary" />
    </Animated.View>
  );
}

interface MessageListProps {
  messages: Message[];
  isLoading: boolean;
  isSending: boolean;
  messageFeedback: Record<string, 'up' | 'down' | null>;
  /** Saved thumbs-down reasons, keyed by message id. */
  messageFeedbackComment: Record<string, string>;
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
  flatListRef: React.RefObject<FlashListRef<ChatRow> | null>;
  onScrollToBottom: () => void;
  onThumbsUp: (messageId: string) => void;
  onThumbsDown: (messageId: string) => void;
  /** Persist an optional thumbs-down reason for a message. */
  onSubmitFeedbackReason: (messageId: string, comment: string) => void;
  onRetryMessage: (messageId: string) => void;
  onOpenUrl: (url: string) => void;
  /**
   * Re-authorize a provider the reply says has fallen off.
   *
   * Named by provider rather than handed the reply's URL: that URL was minted
   * for a browser callback, while the phone re-authorizes inside the app's own
   * authentication session and needs its own return URL. `onOpenUrl` would
   * hand the athlete to Safari and never bring them back.
   */
  onReconnectProvider: (provider: string) => void;
  /** Press handler for a control the reply's `actions` block carried. */
  onActionClick?: (action: ChatMessageAction) => void;
  /**
   * Open the verdict sheet for a message. Receives the rows the surface has
   * for it — possibly none yet, when only the turn's chips have landed, in
   * which case the host fetches them.
   */
  onShowVerdict?: (rows: ClaimVerdict[], messageId: string) => void;
  /**
   * Space to keep clear at the bottom: whichever is taller, the resting
   * composer or the open keyboard. Was a hardcoded 140, which was only ever
   * right with the keyboard closed on a home-indicator phone.
   */
  bottomInset: number;
}

/**
 * The composer pill's own height plus the vertical padding around it. The list
 * has to clear the composer as well as whatever the composer is sitting on.
 */
const COMPOSER_CLEARANCE = 64;

export function MessageList({
  messages,
  isLoading,
  isSending,
  messageFeedback,
  messageFeedbackComment,
  messageBlocks,
  verdicts,
  flatListRef,
  onScrollToBottom,
  onThumbsUp,
  onThumbsDown,
  onSubmitFeedbackReason,
  onRetryMessage,
  onOpenUrl,
  onReconnectProvider,
  onActionClick,
  onShowVerdict,
  bottomInset,
}: MessageListProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const markdownStyles = useMemo(() => buildMarkdownStyles(colors), [colors]);
  const handleCopyMessage = async (content: string) => {
    try {
      await Clipboard.setStringAsync(content);
      Alert.alert(t('app.copiedTitle'), t('app.copiedBody'));
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

  /**
   * The reply's own markdown, with its charts drawn where its markers sit.
   *
   * `linkifyUrls` first, because markdown does not autolink a bare URL: the
   * coach writes the reconnect address as running text and it would render as
   * running text, with nothing to tap.
   */
  const renderProse = (text: string, scenes: RenderBlock[], key: number) => (
    <View key={key}>
      {splitVizMarkers(linkifyUrls(text)).map((segment, i) =>
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
    context: { isUser: boolean; messageId: string; scenes: RenderBlock[]; rows: ClaimVerdict[] },
  ): React.ReactNode => {
    switch (block.type) {
      case 'prose':
        return context.isUser ? (
          // The ink the tint binds. DESIGN.md §2 pairs `primary-container`
          // with `on-primary-container` (9.8:1 light, 7.5:1 dark), so the
          // bubble's ink moves with its ground instead of staying on the
          // body role the canvas uses.
          <Text
            key={key}
            className="text-base leading-6"
            style={{ color: colors.tokens.onPrimaryContainer }}
          >
            {block.text}
          </Text>
        ) : (
          renderProse(block.text, context.scenes, key)
        );

      case 'activity_list':
        return (
          <CollapsibleActivities key={key} activityText={block.text} onLinkPress={onOpenUrl} />
        );

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
            accessibilityLabel={block.caption ?? t('app.chartFallbackAria')}
          />
        );

      case 'verdicts': {
        // The turn's chips preview the verdicts until the rows land; once the
        // rows exist they are the count, so the chip cannot grow when a chip
        // and its row spell the claim differently.
        const severities =
          context.rows.length > 0
            ? context.rows.map((row) => ({ status: row.status, evidence_strength: row.evidence_strength }))
            : block.chips.map(verdictChipSeverity);
        const summary = summarizeVerdicts(severities);
        if (!summary) return null;
        const chip = verdictChipPalette(summary.tone, colors);
        return (
          <TouchableOpacity
            key={key}
            testID="verdict-chip"
            accessibilityRole="button"
            accessibilityLabel={t('app.claimVerdictsStatus', {
              status: t(VERDICT_STATUS_LABEL_KEY[summary.worstStatus]),
            })}
            className="flex-row items-center self-start mt-2 px-2 py-1 rounded-full"
            style={{ backgroundColor: chip.fill }}
            onPress={() => onShowVerdict?.(context.rows, context.messageId)}
          >
            {/* The shield is the tone's other carrier — half-filled when the
                rollup alerts — so it takes the ink the label takes rather than
                the fill's own hue, which at 12pt reads as a smudge. */}
            <Ionicons
              name={verdictToneAlerts(summary.tone) ? 'shield-half-outline' : 'shield-outline'}
              size={12}
              color={chip.ink}
            />
            <Text className="text-xs ml-1" style={{ color: chip.ink }}>
              {verdictChipLabel(t, summary)}
            </Text>
          </TouchableOpacity>
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
            onPress={() => onReconnectProvider(block.provider)}
          >
            <Text className="text-sm font-medium text-on-primary">
              {t('app.reconnect')} {block.display_name}
            </Text>
          </TouchableOpacity>
        );

      // A quota notice is a fact about the turn, not about the message: the
      // usage banner below the transcript shows it once.
      case 'notice':
        return null;
    }
  };

  const renderMessage = (item: Message, groupStart: boolean) => {
    const isUser = item.role === 'user';
    const isError = item.isError === true;
    const rows = (verdicts ?? []).filter((verdict) => verdict.message_id === item.id);
    // One list, whatever the turn's age: the server's own blocks when it just
    // landed, the same shape decoded from the persisted row when it did not.
    const blocks = messageBlocks?.[item.id] ?? transcriptBlocks(item, rows);
    const sceneBlock = blocks.find((block) => block.type === 'scene');
    const scenes = sceneBlock?.type === 'scene' ? parseSceneBlocks(sceneBlock.scene_blocks) : [];
    const context = { isUser, messageId: item.id, scenes, rows };
    // What copy and share hand out. The reply's own text carries the ⟦viz:N⟧
    // markers this surface turns into charts; pasted anywhere else they are a
    // token that means nothing, so each becomes a line naming its chart.
    const readableCopy = copyableText(item.content, scenes, t);

    const clock = item.created_at ? formatMessageTime(item.created_at, language) : '';

    return (
      <View
        testID={groupStart ? 'message-row-start' : 'message-row-continued'}
        className={`${groupStart ? 'mt-3' : 'mt-1'} ${isUser ? 'items-end' : ''}`}
      >
        {isUser ? (
          /* The athlete's message — right-aligned, on the sage tint in both
             schemes, which is what DESIGN.md §5 specifies and what the web
             bubble already wore. A neutral `surface-container-high` made the
             athlete's own words a grey blob on the light canvas and said
             nothing about whose they were; the tint is the same green the
             active rail item and the unread pill carry, so the thread reads
             as a conversation with two sides rather than as grey on paper.
             A FILLED primary is not the alternative — that reads as a call to
             action, not as a message. */
          <View
            className="max-w-[85%] rounded-2xl rounded-br-[4px] px-4 py-3"
            style={{ backgroundColor: colors.tokens.primaryContainer }}
          >
            {blocks.map((block, index) => renderBlock(block, index, context))}
            {clock ? (
              <Text
                className="mt-1 text-right text-xs"
                style={{ color: colors.tokens.onPrimaryContainer, opacity: 0.75 }}
                testID="message-time"
              >
                {clock}
              </Text>
            ) : null}
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
                <Text className="text-xs text-text-primary font-medium">{t('common.retry')}</Text>
              </TouchableOpacity>
            ) : (
              <>
                <TouchableOpacity className="p-0.5" onPress={() => handleCopyMessage(readableCopy)}>
                  <Ionicons name="copy-outline" size={14} color={colors.text.tertiary} />
                </TouchableOpacity>
                <TouchableOpacity className="p-0.5" onPress={() => handleShareMessage(readableCopy)}>
                  <Ionicons name="arrow-redo-outline" size={14} color={colors.text.tertiary} />
                </TouchableOpacity>
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
            {/* The clock belongs to the row, not to what it offers: an error
                row shows Retry and still says when it arrived. */}
            {clock ? (
              <Text className="text-xs text-text-tertiary ml-auto" testID="message-time">
                {clock}
              </Text>
            ) : null}
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

  /**
   * The transcript as rows: a day pill wherever the local date changes, and a
   * run boundary wherever the author or the five-minute window does.
   *
   * Tool plumbing is dropped here rather than inside the renderer, so a
   * `tool_call` row can never sit between two of an author's messages and
   * break the run they belong to. Both decisions read `created_at`, which an
   * optimistic row carries from the moment it was typed.
   */
  const rows = useMemo<ChatRow[]>(() => {
    const out: ChatRow[] = [];
    let previous: Message | null = null;
    let previousDay = '';
    for (const message of filterDisplayMessages(messages ?? [])) {
      if (!message?.id) continue;
      const day = message.created_at ? localDayKey(message.created_at) : '';
      if (day && day !== previousDay) {
        const label = dayLabelFor(message.created_at, language);
        out.push({
          kind: 'day',
          key: `day-${day}`,
          label:
            label.kind === 'today'
              ? t('chat.dayToday')
              : label.kind === 'yesterday'
                ? t('chat.dayYesterday')
                : label.label,
        });
        previousDay = day;
      }
      out.push({
        kind: 'message',
        key: message.id,
        message,
        groupStart: previous === null || !isSameMessageGroup(previous, message),
      });
      previous = message;
    }
    return out;
  }, [messages, language, t]);

  const renderRow = ({ item }: { item: ChatRow }) =>
    item.kind === 'day' ? (
      <DaySeparator label={item.label} />
    ) : (
      renderMessage(item.message, item.groupStart)
    );

  /**
   * The coach is composing.
   *
   * The mark and three breathing dots, hugging their own content: `alignSelf`
   * is what sizes this row, since a plain parent stretches its child across
   * the full width and a message-shaped slab holding 70pt of content reads as
   * a broken bubble rather than as "typing". No bubble chrome for the same
   * reason — there is no message here yet to put in one.
   */
  const renderThinkingIndicator = () => (
    <View
      className="mb-4 flex-row items-center"
      style={{ alignSelf: 'flex-start' }}
      testID="thinking-indicator"
    >
      <View className="w-8 h-8 rounded-full mr-3 overflow-hidden">
        <Image
          source={require('../../../assets/icon.png')}
          className="w-8 h-8"
          resizeMode="cover"
        />
      </View>
      <View className="flex-row items-center gap-1">
        {Array.from({ length: TYPING_DOT_COUNT }, (_, index) => (
          <TypingDot key={index} index={index} />
        ))}
      </View>
    </View>
  );

  /**
   * An empty thread.
   *
   * One line and the two ways in: `/` for the command palette, `@handle` to
   * bring an agent in for a turn. No agent grid, no picker — an agent is chosen
   * with `/agent add @handle`, exactly as it is on web and in messaging.
   */
  const renderEmptyChat = () => (
    <ScrollView
      className="flex-1"
      contentContainerStyle={{
        flexGrow: 1,
        alignItems: 'center',
        justifyContent: 'center',
        paddingHorizontal: spacing.lg,
        paddingBottom: 140,
      }}
      showsVerticalScrollIndicator={false}
      keyboardShouldPersistTaps="handled"
      testID="chat-empty-state"
    >
      <Text className="text-base text-text-secondary text-center leading-6">
        {t('app.emptyThreadPrompt')}
      </Text>
      <Text className="text-sm text-text-tertiary text-center mt-2" testID="chat-slash-hint">
        {t(SLASH_HINT_KEY)}
      </Text>
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
        data={rows}
        renderItem={renderRow}
        keyExtractor={(item) => item.key}
        // A pill and a bubble are different shapes; recycling one as the other
        // is what makes a separator flicker into a message on fast scroll.
        getItemType={(item) => item.kind}

        contentContainerStyle={{
          paddingHorizontal: spacing.md,
          paddingVertical: spacing.md,
          // COMPOSER_CLEARANCE is the pill's own height plus its padding; the
          // inset above it is the resting bar or the raised keyboard.
          paddingBottom: bottomInset + COMPOSER_CLEARANCE,
        }}
        showsVerticalScrollIndicator={false}
        onContentSizeChange={onScrollToBottom}
        ListFooterComponent={isSending ? renderThinkingIndicator : null}
      />
    </View>
  );
}
