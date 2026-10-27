// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A mobile app session minted by `pierre-cli user mint-session`, handed to a Maestro flow as a single-use code
// ABOUTME: Served by the control process (home-sync-control.ts); the app's E2E-only session route redeems the code for the pair

import { randomBytes } from 'node:crypto';
import { runPierreCli, type CliServer } from './pierre-cli';

/** The pair the `dravr-mobile` code exchange would have issued, as the app stores it. */
export interface SeededSession {
  access_token: string;
  refresh_token: string;
}

/**
 * How long a code waits to be redeemed. A flow asks for one right before it
 * opens the link, and the route redeems it as it mounts, so a minute covers a
 * slow simulator; a code still unused by then is worth nothing.
 */
export const SEEDED_SESSION_CODE_TTL_MS = 60_000;

/**
 * Mint a session for `email` the way the hosted sign-in's code exchange does
 * (carnet#1201): the same `dravr-mobile` client, `offline_access` refresh
 * token and expiry, issued by the server's own calls inside the CLI
 * (crates/pierre-cli/src/commands/session.rs). The flows whose subject is not
 * sign-in open the app with it instead of the hosted sign-in sheet, which the
 * CI iOS simulator can leave blank for minutes.
 */
export async function mintSeededSession(server: CliServer, email: string): Promise<SeededSession> {
  if (email === '') {
    throw new Error('a seeded session names the account it signs in (email)');
  }
  const printed = await runPierreCli(server, ['user', 'mint-session', '--email', email]);
  const minted = JSON.parse(printed.trim()) as Partial<SeededSession>;
  if (typeof minted.access_token !== 'string' || typeof minted.refresh_token !== 'string') {
    throw new Error('pierre-cli user mint-session printed no token pair');
  }
  return { access_token: minted.access_token, refresh_token: minted.refresh_token };
}

/**
 * Minted sessions waiting for the app, each behind a short random code.
 *
 * The link a flow opens carries only the code, never the tokens: Maestro
 * writes every opened link to its log, which the lanes upload as a public
 * artifact. A code is redeemed once, and only within
 * {@link SEEDED_SESSION_CODE_TTL_MS}; a second redemption, an expired code or
 * one never issued gets nothing. Kept in this process's memory alone.
 */
export class SeededSessionCodes {
  private readonly pending = new Map<string, { session: SeededSession; expiresAt: number }>();

  constructor(
    private readonly now: () => number = Date.now,
    private readonly ttlMs: number = SEEDED_SESSION_CODE_TTL_MS,
  ) {}

  /** Hold `session` and return the code that redeems it. */
  issue(session: SeededSession): string {
    this.forgetExpired();
    const code = randomBytes(16).toString('base64url');
    this.pending.set(code, { session, expiresAt: this.now() + this.ttlMs });
    return code;
  }

  /** The session `code` was issued for, once: the code is spent whether or not it was still valid. */
  redeem(code: string): SeededSession | undefined {
    const entry = this.pending.get(code);
    this.pending.delete(code);
    if (entry === undefined || this.now() >= entry.expiresAt) {
      return undefined;
    }
    return entry.session;
  }

  private forgetExpired(): void {
    const now = this.now();
    for (const [code, entry] of this.pending) {
      if (now >= entry.expiresAt) {
        this.pending.delete(code);
      }
    }
  }
}
