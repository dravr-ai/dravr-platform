// ABOUTME: Main entry point for @pierre/chat-utils package
// ABOUTME: Re-exports all chat utility functions for convenient importing

// Insight detection and generation utilities
export {
  INSIGHT_PROMPT_PREFIX,
  isInsightPrompt,
  detectInsightMessages,
  createInsightPrompt,
} from './insight';

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

// Turn-progress event → status text mapping (shared by web + mobile)
export { statusTextForProgress, THINKING_PLACEHOLDER } from './progress';

// Inline visual blocks: split a reply on its positional markers, parse the
// resolved scenes. Shared so web and mobile interleave prose and charts the
// same way.
export { splitVizMarkers, parseSceneBlocks, blockAt } from './viz';
export type { VizSegment } from './viz';
