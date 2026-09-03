// ABOUTME: Main entry point for @pierre/chat-utils package
// ABOUTME: Re-exports all chat utility functions for convenient importing

// The trusted-domain gate a `url` reply action passes before a client opens it
export { trustedActionUrl } from './action-url';

// A turn's `notice` block -> the usage banner both clients show
export { quotaNoticeBanner } from './quota';
export type { QuotaBanner } from './quota';

// Persisted transcript row -> the ReplyBlock list a live turn arrives in.
// The command-reply normaliser is private to that decoder: the live turn path
// never called it, and an export nobody reads is a promise nobody keeps.
export { COMMAND_FINISH_REASON, transcriptBlocks } from './blocks';

// Activity list parsing (backward compat for old messages with baked-in content)
export { splitActivityContent, countActivities } from './activity';

// Cross-surface conversation rendering helpers (web + mobile parity):
// hide tool-plumbing rows, strip residual tool XML, derive channel badges.
export {
  isToolPlumbingMessage,
  filterDisplayMessages,
  stripToolScaffolding,
  deriveMessageChannel,
  resolveChannelOrigin,
} from './conversation';
export type { MessageChannelOrigin } from './conversation';

// The unified conversation-list row: one model, one preview rule, one
// timestamp rule, one avatar colour — derived identically on web and mobile.
// The words it cannot spell itself arrive as labels each client resolves
// from CONVERSATION_ROW_LABEL_KEYS with its own t().
export {
  AVATAR_SLOTS,
  CONVERSATION_ROW_LABEL_KEYS,
  conversationRowLabels,
  deriveKind,
  initialsFor,
  avatarSlot,
  previewFor,
  formatListTimestamp,
  defaultConversationTitle,
  buildConversationRow,
  sortRowsByActivity,
  filterRows,
} from './conversation-row';
export type {
  ConversationKind,
  ConversationRowLabels,
  ConversationRowModel,
} from './conversation-row';

// Turn-progress event → status text mapping (shared by web + mobile)
export { statusForProgress, THINKING_PLACEHOLDER_KEY } from './progress';
export type { ProgressStatus } from './progress';
export type { TranslatableText, Translate } from './text';

// Inline visual blocks: split a reply on its positional markers, parse the
// resolved scenes. Shared so web and mobile interleave prose and charts the
// same way.
export { splitVizMarkers, parseSceneBlocks, blockAt } from './viz';
export type { VizSegment } from './viz';

// What a reply becomes on the clipboard or in a share sheet: the same prose,
// with every marker replaced by a line naming the chart that stood there.
export { copyableText } from './copy-text';

// The bubble clock, the day pill and the grouping window of the messenger thread
export {
  MESSAGE_GROUP_WINDOW_MS,
  dayLabelFor,
  formatMessageTime,
  isSameMessageGroup,
  localDayKey,
} from './message-time';
export type { DayLabel } from './message-time';

// One localized date-and-time stamp for everything a surface saved earlier —
// memory facts, verdicts, admin tables.
export { formatDateTime } from './date-format';

// What a chat header says under the title, with one precedence for both clients
export { threadSubtitle } from './thread-subtitle';
export type { ThreadSubtitle } from './thread-subtitle';

// The header's fallback line when there is no group and no handle to name:
// which providers the coach can see, or that none are connected.
export { providerStatusLine } from './provider-status';
export type { ProviderStatusRow } from './provider-status';
