// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Estimates how much of the model's context an agent's system prompt takes
// ABOUTME: One formula and one catalogue sentence for the web agent form and the phone's agent editor

/** The context window the estimate is measured against, in tokens. */
export const PROMPT_CONTEXT_WINDOW_TOKENS = 128_000;

/** Characters per token in the estimate — the usual rule of thumb for English prose. */
const CHARS_PER_TOKEN = 4;

/**
 * The catalogue sentence that reads an estimate out: `~1,234 tokens (1.0% of
 * context)` in English, `~1 234 jetons (1,0 % du contexte)` in French. Its
 * `tokens` and `percent` slots are numbers the catalogue formats in the
 * active language, so pass {@link PromptTokenEstimate} as it comes.
 */
export const PROMPT_TOKEN_ESTIMATE_KEY = 'chat.systemPromptTokenEstimate';

/**
 * A prompt's estimated size. A type alias rather than an interface, so it can
 * be handed to `t()` as its interpolation options as it comes.
 */
export type PromptTokenEstimate = {
  /** Estimated tokens, rounded up. */
  tokens: number;
  /** Share of {@link PROMPT_CONTEXT_WINDOW_TOKENS}, as a percentage (0–100, uncapped). */
  percent: number;
};

/** Estimate the tokens `prompt` takes and its share of the context window. */
export function estimatePromptTokens(prompt: string): PromptTokenEstimate {
  const tokens = Math.ceil(prompt.length / CHARS_PER_TOKEN);
  return { tokens, percent: (tokens / PROMPT_CONTEXT_WINDOW_TOKENS) * 100 };
}
