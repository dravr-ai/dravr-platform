// ABOUTME: Main entry point for @pierre/chat-utils package
// ABOUTME: Re-exports all chat utility functions for convenient importing

// The trusted-domain gate a `url` reply action passes before a client opens it
export { trustedActionUrl } from './action-url';

// A turn's `notice` block -> the usage banner both clients show
export { quotaNoticeBanner } from './quota';
export type { QuotaBanner } from './quota';

// Persisted transcript row -> the ReplyBlock list a live turn arrives in
export { transcriptBlocks } from './blocks';

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
export {
  AVATAR_SLOTS,
  UNTITLED_CONVERSATION,
  deriveKind,
  initialsFor,
  avatarSlot,
  previewFor,
  formatListTimestamp,
  buildConversationRow,
  sortRowsByActivity,
  filterRows,
} from './conversation-row';
export type { ConversationKind, ConversationRowModel } from './conversation-row';

// Turn-progress event → status text mapping (shared by web + mobile)
export { statusTextForProgress, THINKING_PLACEHOLDER } from './progress';

// Inline visual blocks: split a reply on its positional markers, parse the
// resolved scenes. Shared so web and mobile interleave prose and charts the
// same way.
export { splitVizMarkers, parseSceneBlocks, blockAt } from './viz';
export type { VizSegment } from './viz';
