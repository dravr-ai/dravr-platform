// ABOUTME: Slash-command palette matching — one rule for when it opens and what it lists
// ABOUTME: Shared by web and mobile so a "/" means the same thing on both composers

import type { CommandEntry } from '@pierre/shared-types';

/** The character that opens the palette. */
export const COMMAND_PREFIX = '/';

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
