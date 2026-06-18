// ABOUTME: Utility functions for message content processing
// ABOUTME: Text transformations and formatting for chat messages

/**
 * Regular expression to match internal context prefixes in messages.
 * Format: [Context:...] at the start of a message
 */
const CONTEXT_PREFIX_REGEX = /^\[Context:[^\]]*\]\s*/i;

/**
 * Strip internal context prefixes from message content before displaying to user.
 * Context prefixes are added by the system for internal use but should not be shown in UI.
 *
 * @param text - The message text that may contain a context prefix
 * @returns The text with any context prefix removed
 *
 * @example
 * stripContextPrefix("[Context:coach] Hello there") // "Hello there"
 * stripContextPrefix("Hello there") // "Hello there"
 */
export const stripContextPrefix = (text: string): string => {
  return text.replace(CONTEXT_PREFIX_REGEX, '');
};

/**
 * Inputs for {@link buildOutgoingMessage}: the provider context the web/mobile
 * chat clients can attach to a user's message.
 */
export interface ProviderContextInput {
  /** Provider name from a just-completed OAuth connect, if any. */
  justConnectedProvider?: string | null;
  /** Comma-joined display names of connected providers, for the first turn. */
  connectedProviders?: string | null;
  /** Whether this is the first message of the conversation. */
  isFirstMessage: boolean;
}

/**
 * Build the message body to send to the chat API, optionally prefixed with an
 * internal `[Context:...]` hint so the coach knows which providers are linked.
 *
 * Slash commands are returned verbatim. The backend command dispatcher only
 * recognizes a command when the leading `/` is at position 0; prepending a
 * context prefix would push the slash off the front and the command would fall
 * through to the LLM instead of dispatching. The inverse of
 * {@link stripContextPrefix}.
 *
 * @example
 * buildOutgoingMessage("/context", { isFirstMessage: true, connectedProviders: "Strava" }) // "/context"
 * buildOutgoingMessage("hi", { isFirstMessage: true, connectedProviders: "Strava" }) // "[Context: I have connected Strava] hi"
 */
export const buildOutgoingMessage = (
  content: string,
  ctx: ProviderContextInput,
): string => {
  if (content.startsWith('/')) {
    return content;
  }
  if (ctx.justConnectedProvider) {
    return `[Context: I just connected my ${ctx.justConnectedProvider} account successfully] ${content}`;
  }
  if (ctx.isFirstMessage && ctx.connectedProviders) {
    return `[Context: I have connected ${ctx.connectedProviders}] ${content}`;
  }
  return content;
};
