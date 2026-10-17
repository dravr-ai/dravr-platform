// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The right drawer behind the thread header — Group info, Agent info, a plan thread's athlete, or a plain thread's own controls
// ABOUTME: Tapping the header is how App Messaging reaches this, so nothing here needs a tab of its own

import { useEffect, useState } from 'react';
import { clsx } from 'clsx';
import { Trash2, Users } from 'lucide-react';
import type { Conversation } from '@pierre/shared-types';
import { avatarSlot, initialsFor, PLAN_THREAD_KEYS } from '@pierre/chat-utils';
import { Button, Input } from '../ui';
import { useCoachInfo } from '../../hooks/useCoachInfo';
import { useGroup } from '../../hooks/useGroups';
import { avatarShapeClass, avatarSlotClass } from './avatarSlots';
import GroupInfoPanel from '../groups/GroupInfoPanel';
import CoachInfoPanel from './CoachInfoPanel';
import ConversationParticipants from './ConversationParticipants';
import { useTranslation } from '@pierre/i18n';

interface ConversationInfoPanelProps {
  /** The open conversation, as the list serves it. */
  conversation: Conversation;
  /** Dismiss the drawer. */
  onClose: () => void;
  /** Send a turn in this conversation — how the agent commands are issued. */
  onSendCommand: (text: string) => void;
  /** Open an agent's Discover edit sheet, `discover/<agentId>`. */
  onEditCoach: (agentId: string) => void;
  /** Rename this conversation. */
  onRename: (title: string) => void;
  /** Delete this conversation. */
  onDelete: () => void;
  /**
   * The thread is no longer the caller's — they left the group or archived
   * it. The host drops the selection along with the drawer.
   */
  onThreadGone: () => void;
  /** Open with the participants control already expanded. */
  openParticipants?: boolean;
  /** Open the connections pane, where a group's coach connects TrainingPeaks. */
  onOpenConnections?: () => void;
  /** Open another conversation — where Group info sends a coach to an athlete's plan thread. */
  onOpenConversation?: (conversationId: string) => void;
}

/** The shapes this drawer takes. */
type InfoShape = 'group' | 'subject' | 'coach' | 'plain';

/**
 * What the open conversation makes this drawer about. A coach's plan thread
 * runs on its group's agent, but it is about the athlete, so it is checked
 * before the agent.
 */
function shapeOf(conversation: Conversation): InfoShape {
  if (conversation.group_id) return 'group';
  if (conversation.subject_user_id) return 'subject';
  if (conversation.agent_id) return 'coach';
  return 'plain';
}

// Built at import time, where `t` does not exist: the table carries the key
// and the render resolves it.
const HEADING_KEYS: Record<InfoShape, string> = {
  group: 'chat.infoPanelGroupTitle',
  subject: PLAN_THREAD_KEYS.kindLabel,
  coach: 'chat.agentInfo',
  plain: 'chat.infoPanelChatTitle',
};

/**
 * A coach's plan thread, described: the athlete whose plan it works on, and
 * the group whose coaching link scopes it. Nothing here manages the group —
 * that is Group info's, in the group's own thread.
 */
function SubjectInfo({ conversation }: { conversation: Conversation }) {
  const { t } = useTranslation();
  const { group } = useGroup(conversation.subject_group_id ?? '');
  const name = conversation.subject_display_name?.trim() || t('app.unknownMember');
  return (
    <div className="space-y-5" data-testid="subject-info-panel">
      <section className="flex items-center gap-3">
        <span
          aria-hidden="true"
          className={clsx(
            'flex h-10 w-10 shrink-0 select-none items-center justify-center text-sm font-semibold',
            avatarShapeClass('circle'),
            avatarSlotClass(avatarSlot(conversation)),
          )}
        >
          {initialsFor(name)}
        </span>
        <h3 className="min-w-0 truncate text-lg font-semibold text-on-surface" data-testid="subject-info-name">
          {name}
        </h3>
      </section>
      {group && (
        <section className="space-y-1">
          <h4 className="flex items-center gap-1.5 text-xs font-semibold text-outline">
            <Users className="h-3.5 w-3.5" aria-hidden="true" />
            {t('app.group')}
          </h4>
          <p className="text-sm text-on-surface" data-testid="subject-info-group">
            {group.name}
          </p>
        </section>
      )}
    </div>
  );
}

/**
 * Everything there is to know about, and do to, the open thread.
 *
 * Four shapes, one drawer: a group thread gets the whole re-homed group
 * surface, a coach's plan thread gets its athlete and their group, an agent
 * thread gets the agent and its two command-backed actions, and a plain
 * thread gets its own name, its participants and its delete. The
 * shape is read off the conversation, never passed in, so a thread that gains
 * an agent through `/agent add` changes shape on the next list refetch.
 */
export default function ConversationInfoPanel({
  conversation,
  onClose,
  onSendCommand,
  onEditCoach,
  onRename,
  onDelete,
  onThreadGone,
  openParticipants = false,
  onOpenConnections,
  onOpenConversation,
}: ConversationInfoPanelProps) {
  const { t } = useTranslation();
  const shape = shapeOf(conversation);
  const { coach } = useCoachInfo(shape === 'coach' ? conversation.agent_id : null);
  const [title, setTitle] = useState(conversation.title ?? '');
  const [participantsOpen, setParticipantsOpen] = useState(openParticipants);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  const trimmed = title.trim();
  const renameDisabled = !trimmed || trimmed === (conversation.title ?? '');

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-end bg-black/60"
      role="dialog"
      aria-modal="true"
      aria-label={t(HEADING_KEYS[shape])}
      onClick={onClose}
    >
      <div
        className="h-full w-full max-w-md overflow-y-auto bg-surface-container-lowest text-on-surface pad-safe-top"
        onClick={(e) => e.stopPropagation()}
        data-testid="conversation-info-panel"
      >
        <div className="sticky top-0 z-10 flex items-start justify-between border-b ghost-border bg-surface-container-lowest/90 px-5 py-4 backdrop-blur">
          <h3 className="text-lg font-semibold text-on-surface">{t(HEADING_KEYS[shape])}</h3>
          <button
            type="button"
            onClick={onClose}
            className="rounded p-1 text-on-surface-variant hover:bg-surface-container hover:text-on-surface"
            aria-label={t('chat.close')}
          >
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        </div>

        <div className="px-5 py-5">
          {shape === 'group' && conversation.group_id ? (
            <GroupInfoPanel
              groupId={conversation.group_id}
              onMembershipEnded={onThreadGone}
              onOpenConnections={onOpenConnections}
              onOpenConversation={onOpenConversation}
            />
          ) : shape === 'subject' ? (
            <SubjectInfo conversation={conversation} />
          ) : shape === 'coach' ? (
            coach ? (
              <CoachInfoPanel
                coach={coach}
                onSendCommand={onSendCommand}
                onEditCoach={onEditCoach}
              />
            ) : (
              <p className="py-6 text-center text-sm text-outline" data-testid="coach-info-missing">
                {t('chat.agentInfoLoadFailed')}
              </p>
            )
          ) : (
            <div className="space-y-6" data-testid="plain-info-panel">
              <section className="space-y-2">
                <Input
                  label={t('chat.chatNameLabel')}
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  maxLength={200}
                  data-testid="conversation-info-title"
                />
                <div className="flex justify-end">
                  <Button
                    variant="primary"
                    size="sm"
                    disabled={renameDisabled}
                    onClick={() => onRename(trimmed)}
                    data-testid="conversation-info-rename"
                  >
                    {t('chat.renameAction')}
                  </Button>
                </div>
              </section>

              <section>
                <h4 className="mb-2 text-xs font-semibold text-outline">
                  {t('chat.participantsHeading')}
                </h4>
                <ConversationParticipants
                  conversationId={conversation.id}
                  open={participantsOpen}
                  onOpenChange={setParticipantsOpen}
                />
              </section>

              <section className="space-y-3 rounded-lg border border-error/20 p-4">
                <h4 className="text-xs font-semibold text-error">
                  {t('chat.dangerZone')}
                </h4>
                <div className="flex items-center justify-between gap-3">
                  <p className="text-sm text-on-surface">{t('chat.deleteChatHint')}</p>
                  <Button
                    variant="danger"
                    size="sm"
                    onClick={onDelete}
                    data-testid="conversation-info-delete"
                  >
                    <span className="flex items-center gap-1.5">
                      <Trash2 className="w-4 h-4" aria-hidden="true" />
                      {t('chat.deleteAction')}
                    </span>
                  </Button>
                </div>
              </section>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
