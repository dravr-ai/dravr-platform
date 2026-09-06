// ABOUTME: Slash-command palette matching — one rule for when it opens and what it lists
// ABOUTME: Shared by web and mobile so a "/" means the same thing on both composers

import type { CommandEntry } from '@pierre/shared-types';
import { MENTION_PREFIX } from './mentions';

/** The character that opens the palette. */
export const COMMAND_PREFIX = '/';

/**
 * The composer text each client-side affordance hands to the composer.
 *
 * Every entry names a command the catalogue serves, spelled exactly as the
 * parser expects it, so a hint card, a "New group chat" prompt, an invite deep
 * link and a swipe action all produce the same line an athlete would type.
 * One spelling here, on both clients, is what keeps a renamed command from
 * silently stranding one of them.
 */
export const COMMAND_DRAFTS = {
  /** Binds an installed agent to the open conversation. */
  coachAdd: (handle: string): string => `${COMMAND_PREFIX}agent add ${MENTION_PREFIX}${handle}`,
  /** Detaches the conversation's agent. */
  coachRemove: `${COMMAND_PREFIX}agent remove`,
  /** Creates a coaching group and binds the thread to it. */
  groupCreate: (name: string): string => `${COMMAND_PREFIX}group create ${name}`,
  /** Joins a coaching group by invite code. */
  groupJoin: (code: string): string => `${COMMAND_PREFIX}group join ${code}`,
  /** Brings an agent in for one turn; the trailing space lets the athlete keep typing. */
  mention: (handle: string): string => `${MENTION_PREFIX}${handle} `,
} as const;

/**
 * The corpus key of the one line an empty thread shows beside the composer —
 * the discoverable path to the command palette and the mention grammar, on
 * every client. A constants module cannot translate, so it names the key and
 * each client resolves it with its own `t()`.
 */
export const SLASH_HINT_KEY = 'chat.slashHint';

/**
 * Whether this composer text is a command draft at all.
 *
 * A draft is text whose first non-space character is `/` and which has not yet
 * been broken by a newline — once the athlete starts a second line they are
 * writing prose, whatever the first character was.
 */
export function isCommandDraft(value: string): boolean {
  return value.trimStart().startsWith(COMMAND_PREFIX) && !value.includes('\n');
}

/**
 * The commands to offer for this composer text, in catalogue order.
 *
 * Two ways to match, and both are needed:
 *
 * - the athlete is still typing the command, so the draft is a prefix of it
 *   (`/gr` offers `/group status`, `/group invite`, …);
 * - the athlete has finished the command and started its arguments, so the
 *   command is a prefix of the draft (`/plan we` keeps `/plan` on screen with
 *   its `[week|today]` signature visible).
 *
 * Returns an empty array for anything that is not a command draft, which is
 * what closes the palette — there is no separate open/closed flag to drift.
 */
export function matchCommands(catalogue: CommandEntry[], value: string): CommandEntry[] {
  if (!isCommandDraft(value)) return [];
  const draft = value.trimStart().toLowerCase();
  return catalogue.filter((entry) => {
    const command = entry.command.toLowerCase();
    return command.startsWith(draft) || draft.startsWith(`${command} `);
  });
}

/**
 * The composer text that selecting `entry` produces.
 *
 * A command that takes arguments gets a trailing space so the athlete types
 * straight into them; one that takes none does not, so pressing Enter sends it
 * exactly as the parser expects.
 */
export function commandDraftFor(entry: CommandEntry): string {
  return entry.args === null ? entry.command : `${entry.command} `;
}

/**
 * The catalogue key naming a command domain — the badge beside each palette
 * entry — from the domain slug the catalogue route returns. The same keys
 * head the domains in `/help`, so the palette and the help card agree.
 */
export const COMMAND_DOMAIN_LABEL_KEY: Record<string, string> = {
  account: 'commands.help.domain.account',
  coach: 'commands.help.domain.coach',
  data: 'commands.help.domain.data',
  discover: 'commands.help.domain.discover',
  general: 'commands.help.domain.general',
  group: 'commands.help.domain.group',
  provider: 'commands.help.domain.provider',
  training: 'commands.help.domain.training',
};

/** The label key for `domain`, or `null` for a slug the catalogue has no word for. */
export function commandDomainLabelKey(domain: string): string | null {
  return COMMAND_DOMAIN_LABEL_KEY[domain] ?? null;
}
