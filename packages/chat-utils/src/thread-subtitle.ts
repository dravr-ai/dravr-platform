// ABOUTME: What a chat header says under the thread title, decided once for both clients
// ABOUTME: A plan thread's athlete, then group, then coach handle, then nothing — the precedence the two headers disagreed on

import type { Conversation } from '@pierre/shared-types';

/**
 * What the header shows beneath a thread's title.
 *
 * `null` means the thread itself says nothing, and the client falls back to
 * whatever it knows — the web header shows connected-provider status there.
 */
export type ThreadSubtitle =
  | { kind: 'subject'; name: string }
  | { kind: 'group' }
  | { kind: 'handle'; handle: string }
  | null;

/**
 * The subtitle rule: a coach's plan thread names the athlete whose plan it
 * is (carnet#1070), a group thread announces itself as one, otherwise the
 * coach's handle, otherwise nothing.
 *
 * Both clients derived this themselves and disagreed on the order — web read
 * the group badge first, mobile the handle — so one thread bound to a group
 * *and* a coach described itself two different ways depending on the device.
 * Group wins over the handle, because it is the fact about the room; the
 * coach is a participant in it. A plan thread runs on its group's agent, so
 * its handle would name the same agent on every athlete's thread: the
 * athlete is what the coach needs to read there.
 */
export function threadSubtitle(
  conversation:
    | Pick<Conversation, 'group_name' | 'agent_handle' | 'subject_user_id' | 'subject_display_name'>
    | null
    | undefined,
): ThreadSubtitle {
  if (!conversation) return null;
  const subject = conversation.subject_display_name?.trim();
  if (conversation.subject_user_id && subject) return { kind: 'subject', name: subject };
  if (conversation.group_name) return { kind: 'group' };
  if (conversation.agent_handle) {
    return { kind: 'handle', handle: conversation.agent_handle };
  }
  return null;
}
