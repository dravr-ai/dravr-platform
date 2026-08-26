// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pick one of the athlete's coaching groups and open a conversation scoped to it
// ABOUTME: The chat-side entry point to group chat; the conversation itself is started the way GroupDetail starts it

import { Users } from 'lucide-react';
import { Button, Modal, useErrorToast } from '../ui';
import { useMyGroups, useStartGroupConversation } from '../../hooks/useGroups';

export interface GroupChatPickerProps {
  isOpen: boolean;
  onClose: () => void;
  /** Called with the id of the conversation that was just opened. */
  onStarted: (conversationId: string) => void;
  /** Takes the athlete to the Groups surface when they belong to none yet. */
  onGoToGroups?: () => void;
}

/**
 * The "new group chat" sheet.
 *
 * Lists every coaching group the athlete belongs to and opens a conversation
 * bound to the chosen one — `group_id` on the conversation is what turns on
 * the roster, group context and consent-gated peer grounding server-side.
 * One implementation behind two entry points: GroupDetail's "Open group chat"
 * runs the same mutation.
 */
export default function GroupChatPicker({
  isOpen,
  onClose,
  onStarted,
  onGoToGroups,
}: GroupChatPickerProps) {
  const { groups, isLoading, isError } = useMyGroups();
  const { startConversation, isPending } = useStartGroupConversation();
  const showError = useErrorToast();

  const pick = async (groupId: string, groupName: string, coachId: string) => {
    try {
      const conversation = await startConversation({ groupId, groupName, coachId });
      onClose();
      onStarted(conversation.id);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to open group chat';
      showError('Chat failed', message);
    }
  };

  return (
    <Modal isOpen={isOpen} onClose={onClose} title="New group chat" size="md">
      {isLoading ? (
        <div className="flex justify-center py-8">
          <div className="pierre-spinner" />
        </div>
      ) : isError ? (
        <p className="text-sm text-error py-4">Your groups could not be loaded. Try again in a moment.</p>
      ) : groups.length === 0 ? (
        <div className="text-center py-6">
          <div className="w-12 h-12 mx-auto mb-3 rounded-full bg-surface-container-low flex items-center justify-center">
            <Users className="w-6 h-6 text-outline" aria-hidden="true" />
          </div>
          <p className="text-sm text-on-surface mb-1">You are not in a coaching group yet.</p>
          <p className="text-xs text-on-surface-variant mb-4">
            Create or join one from Groups, then start a chat with its coach here.
          </p>
          {onGoToGroups && (
            <Button
              variant="secondary"
              onClick={() => {
                onClose();
                onGoToGroups();
              }}
            >
              Go to Groups
            </Button>
          )}
        </div>
      ) : (
        <ul className="space-y-1" aria-label="Your coaching groups">
          {groups.map((group) => (
            <li key={group.id}>
              <button
                type="button"
                onClick={() => void pick(group.id, group.name, group.coach_id)}
                disabled={isPending}
                className="w-full flex items-center gap-3 px-3 py-3 rounded-lg text-left hover:bg-surface-container-low transition-colors disabled:opacity-50 min-h-[48px]"
              >
                <div className="w-9 h-9 rounded-full bg-primary/10 text-primary flex items-center justify-center flex-shrink-0">
                  <Users className="w-4 h-4" aria-hidden="true" />
                </div>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium text-on-surface truncate">{group.name}</p>
                  <p className="text-xs text-on-surface-variant">
                    {group.member_count} {group.member_count === 1 ? 'member' : 'members'} · {group.my_role}
                  </p>
                </div>
              </button>
            </li>
          ))}
        </ul>
      )}
    </Modal>
  );
}
