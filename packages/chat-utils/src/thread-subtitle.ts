// ABOUTME: What a chat header says under the thread title, decided once for both clients
// ABOUTME: Group before coach handle before nothing — the precedence the two headers disagreed on

import type { Conversation } from '@pierre/shared-types';

/**
 * What the header shows beneath a thread's title.
 *
 * `null` means the thread itself says nothing, and the client falls back to
 * whatever it knows — the web header shows connected-provider status there.
 */
export type ThreadSubtitle =
  | { kind: 'group' }
  | { kind: 'handle'; handle: string }
  | null;

/**
 * The subtitle rule: a group thread announces itself as one, otherwise the
 * coach's handle, otherwise nothing.
 *
 * Both clients derived this themselves and disagreed on the order — web read
 * the group badge first, mobile the handle — so one thread bound to a group
 * *and* a coach described itself two different ways depending on the device.
 * Group wins, because it is the fact about the room; the coach is a
 * participant in it.
 */
export function threadSubtitle(
  conversation: Pick<Conversation, 'group_name' | 'coach_handle'> | null | undefined,
): ThreadSubtitle {
  if (!conversation) return null;
  if (conversation.group_name) return { kind: 'group' };
  if (conversation.coach_handle) {
    return { kind: 'handle', handle: conversation.coach_handle };
  }
  return null;
}
