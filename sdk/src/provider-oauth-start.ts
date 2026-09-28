// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Starts a provider's OAuth flow through Dravr's connect_provider MCP tool before any browser opens
// ABOUTME: Reads the tool's minted authorization URL, and its notice refusal (WHOOP's owner authorization) as a message

import { PierreError, PierreErrorCode } from "./errors.js";
import type { McpCallToolResult } from "./mcp-http-client.js";

/**
 * `error_type` on `connect_provider`'s answer when the account has not accepted the
 * provider's notice (`notice_required_result` in
 * `crates/pierre-tool-runtime/src/implementations/connection.rs`).
 */
export const NOTICE_REQUIRED_ERROR_TYPE = "provider_notice_required";

/** What starting a provider's OAuth flow came to. */
export type ProviderOAuthStart =
  /** Dravr minted the provider's authorization page for the calling athlete: open it. */
  | { kind: "authorize"; url: string }
  /** The account owes the provider's notice: tell the user where to accept it. */
  | { kind: "notice_required"; message: string };

/** Calls one Dravr MCP tool over the bridge's own session. */
export type CallDravrTool = (params: {
  name: string;
  arguments: Record<string, unknown>;
}) => Promise<McpCallToolResult>;

/**
 * The message a user reads when a provider's notice stands between them and
 * its OAuth flow. The notice is accepted by ticking it where it is shown — the
 * Dravr app's connect screens — never through this bridge.
 */
export function providerNoticeMessage(provider: string): string {
  const brand = provider.toUpperCase();
  return (
    `Connecting ${brand} needs your authorization first.\n\n` +
    `Open the Dravr app, go to Connections and connect ${brand}: it shows ${brand}'s notice with a box to tick. ` +
    `Once you have accepted it there, ask me to connect ${brand} again.`
  );
}

/** The text a tool answer carries, for a refusal the caller reports as it is. */
function answerText(result: McpCallToolResult): string {
  const text = result.content
    ?.map((block) => (block as { text?: unknown }).text)
    .filter((part): part is string => typeof part === "string")
    .join(" ");
  return text && text.length > 0 ? text : "no reason given";
}

/**
 * Start `provider`'s OAuth flow by calling Dravr's `connect_provider` tool over the
 * bridge's MCP session.
 *
 * The tool mints the provider's authorization URL for the athlete the session's
 * credential names, and for no one else: it takes no user id, and the flow's state is
 * bound to that athlete and tenant on Dravr. Every credential the bridge can hold is
 * accepted there - a session token, an API key, or a delegated OAuth grant, whose
 * scopes MCP dispatch enforces (`profile:write`) - so one path serves every auth mode.
 *
 * A notice refusal becomes [`providerNoticeMessage`]. Any other refusal, or an answer
 * without an authorization URL, is thrown as a provider error naming what Dravr said.
 */
export async function startProviderOAuth(
  callTool: CallDravrTool,
  provider: string,
): Promise<ProviderOAuthStart> {
  const result = await callTool({ name: "connect_provider", arguments: { provider } });
  const structured = result.structuredContent as
    | { authorization_url?: unknown; error_type?: unknown }
    | null
    | undefined;

  if (result.isError) {
    if (structured?.error_type === NOTICE_REQUIRED_ERROR_TYPE) {
      return { kind: "notice_required", message: providerNoticeMessage(provider) };
    }
    throw new PierreError(
      PierreErrorCode.PROVIDER_ERROR,
      `Dravr refused to start ${provider} authorization: ${answerText(result)}`,
    );
  }

  if (typeof structured?.authorization_url !== "string") {
    throw new PierreError(
      PierreErrorCode.PROVIDER_ERROR,
      `Dravr answered the ${provider} authorization request without an authorization_url`,
    );
  }
  return { kind: "authorize", url: structured.authorization_url };
}
