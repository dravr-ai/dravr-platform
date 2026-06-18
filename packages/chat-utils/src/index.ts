// ABOUTME: Main entry point for @pierre/chat-utils package
// ABOUTME: Re-exports all chat utility functions for convenient importing

// Insight detection and generation utilities
export {
  INSIGHT_PROMPT_PREFIX,
  isInsightPrompt,
  detectInsightMessages,
  createInsightPrompt,
} from './insight';

// Message processing utilities
export { stripContextPrefix, buildOutgoingMessage } from './message';
export type { ProviderContextInput } from './message';

// Activity list parsing (backward compat for old messages with baked-in content)
export { splitActivityContent, countActivities } from './activity';

// AG-UI progress event → status text mapping (shared by web + mobile)
export {
  statusTextForAguiEvent,
  isTerminalAguiEvent,
  type AguiEventWire,
} from './agui';
