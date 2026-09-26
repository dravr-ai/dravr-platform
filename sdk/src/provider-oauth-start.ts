// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Asks the server to start a provider's OAuth flow before any browser opens
// ABOUTME: Reads the notice refusal (WHOOP's owner authorization) as a message to accept it in the app

/**
 * `details.action` on the server's refusal of a connect whose provider notice
 * the account has not accepted (`NOTICE_REFUSAL_ACTION` in
 * `crates/pierre-services/src/provider_notice.rs`).
 */
export const NOTICE_REFUSAL_ACTION = "accept_provider_notice";

/** What starting a provider's OAuth flow came to. */
export type ProviderOAuthStart =
  /** The server minted the provider's authorization page: open it. */
  | { kind: "authorize"; url: string }
  /** The account owes the provider's notice: tell the user where to accept it. */
  | { kind: "notice_required"; message: string }
  /** Anything else: open the launch route and let the browser show it. */
  | { kind: "open_launch" };

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

/** Whether a response body is the server's notice refusal. */
export function isNoticeRefusal(status: number, body: unknown): boolean {
  if (status !== 400 || typeof body !== "object" || body === null) {
    return false;
  }
  const details = (body as { details?: unknown }).details;
  return (
    typeof details === "object" &&
    details !== null &&
    (details as { action?: unknown }).action === NOTICE_REFUSAL_ACTION
  );
}

/**
 * Start `provider`'s OAuth flow on Dravr's launch route with the bridge's own
 * bearer, without following its redirect.
 *
 * A 302 names the provider's authorization page, which the caller opens
 * directly, so the flow's state is minted once. A notice refusal becomes
 * [`providerNoticeMessage`] instead of a raw 400 in the user's browser. Any
 * other answer, or a request that fails outright, leaves the caller to open
 * the launch route itself.
 */
export async function startProviderOAuth(
  launchUrl: string,
  accessToken: string,
  provider: string,
  fetchImpl: typeof fetch = fetch,
): Promise<ProviderOAuthStart> {
  let response: Response;
  try {
    response = await fetchImpl(launchUrl, {
      method: "GET",
      redirect: "manual",
      headers: { Authorization: `Bearer ${accessToken}` },
    });
  } catch {
    return { kind: "open_launch" };
  }

  const location = response.headers.get("location");
  if (response.status >= 300 && response.status < 400 && location) {
    return { kind: "authorize", url: location };
  }

  let body: unknown = null;
  try {
    body = await response.json();
  } catch {
    body = null;
  }
  if (isNoticeRefusal(response.status, body)) {
    return { kind: "notice_required", message: providerNoticeMessage(provider) };
  }
  return { kind: "open_launch" };
}
