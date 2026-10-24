// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Groups tab: the athlete's rooms beside the open one, its header and info drawer
// ABOUTME: Agents and groups are commands here — no agent CRUD, no group picker, no welcome grid; the thread itself is ChatThread

import { useCallback, useEffect, useMemo, useState } from 'react';
import { Users } from 'lucide-react';
import { avatarSlot, deriveKind, initialsFor, scopeOfKind } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { chatApi } from '../services/api';
import { ChatComposeMenu, NewGroupDialog } from './chat';
import ChatShell from './chat/ChatShell';
import ChatThread from './chat/ChatThread';
import ThreadHeader from './chat/ThreadHeader';
import ThreadConnectBanner from './chat/ThreadConnectBanner';
import ThreadInfoDrawer from './chat/ThreadInfoDrawer';
import { useThreadInfo } from './chat/useThreadInfo';
import { useChatThread, type OpenThread } from './chat/useChatThread';
import type { PendingComposerAction } from './chat/types';
import ConversationList from './dashboard/ConversationList';
import { useIsDesktop } from '../hooks/useBreakpoint';
import { useConversationList } from '../hooks/useConversationList';
import { Button } from './ui';
import { NotificationBell } from './notifications/NotificationBell';

interface ChatTabProps {
  selectedConversation: string | null;
  onSelectConversation: (id: string | null) => void;
  /**
   * Dashboard route navigator, `tab[/subview]`. Editing an agent leaves for
   * `discover/<agentId>`, which is where the edit sheet lives.
   */
  onNavigate?: (route: string) => void;
  /**
   * A thread whose info drawer the shell wants open, once that thread is the
   * one on screen — `#chat/<id>/info`, where a group link proposal waits.
   */
  openInfoFor?: string | null;
  /** Called once the drawer above has been opened. */
  onOpenInfoConsumed?: () => void;
  /** Text the shell wants drafted or sent in a thread — the invite deep link uses it. */
  pendingComposerAction?: PendingComposerAction | null;
  /** Called once the action above has been drafted or dispatched. */
  onPendingComposerActionConsumed?: () => void;
}

/** The Groups tab creates a thread for a turn sent with none open; the server names it. */
const createThread: OpenThread = (request) => chatApi.createConversation(request);

export default function ChatTab({
  selectedConversation,
  onSelectConversation,
  onNavigate,
  openInfoFor = null,
  onOpenInfoConsumed,
  pendingComposerAction,
  onPendingComposerActionConsumed,
}: ChatTabProps) {
  const { t } = useTranslation();
  const thread = useChatThread({
    conversationId: selectedConversation,
    onSelectConversation,
    openThread: createThread,
    pendingComposerAction,
    onPendingComposerActionConsumed,
  });
  const { runComposerAction, activeConversation } = thread;
  const info = useThreadInfo(selectedConversation);
  // The empty Groups tab's own way to start a room.
  const [newGroupOpen, setNewGroupOpen] = useState(false);
  // Below the desktop breakpoint the list hides behind the open thread, so
  // the header carries the way back to it.
  const isDesktop = useIsDesktop();

  // A link that names a thread's info opens its drawer once that thread is on
  // screen. Declared after the drawer's own close on a change of thread
  // (`useThreadInfo` above), so a change of thread that brings the request
  // closes the old drawer and then opens this one.
  const { show: showInfo } = info;
  useEffect(() => {
    if (openInfoFor === null || openInfoFor !== selectedConversation) return;
    showInfo(false);
    onOpenInfoConsumed?.();
  }, [openInfoFor, selectedConversation, onOpenInfoConsumed, showInfo]);

  // Whether the athlete is in any room at all — the Groups tab with none says
  // so in place of an empty list beside an empty thread. Only a list read in
  // full can say so: a failed read, or rooms on a page not loaded yet, leave
  // the list to show its own error or its next page.
  const {
    conversations,
    isLoading: conversationsLoading,
    isError: conversationsFailed,
    hasMore: moreConversations,
  } = useConversationList();
  const hasNoGroups = useMemo(
    () =>
      !conversationsLoading &&
      !conversationsFailed &&
      !moreConversations &&
      !conversations.some((conversation) => scopeOfKind(deriveKind(conversation)) === 'groups'),
    [conversations, conversationsLoading, conversationsFailed, moreConversations],
  );

  const sendCommand = useCallback(
    (command: string) => runComposerAction({ kind: 'send', text: command }),
    [runComposerAction],
  );

  /**
   * The "+" menu, wired the same way in every header slot. The Groups tab
   * offers rooms only — a one-to-one thread is Home's to start.
   */
  const composeMenu = (withParticipants: boolean) => (
    <ChatComposeMenu
      onNewGroupChat={sendCommand}
      onAddParticipant={withParticipants ? () => info.show(true) : undefined}
      disabled={thread.startingThread}
    />
  );

  // The bell sits at the top right of every main surface (carnet#820): on
  // Groups, the end of the list's header, where Home's sits in its own.
  const bell = <NotificationBell onNavigate={onNavigate} />;

  // The Groups tab with no room at all: one sentence and one way to start one,
  // in place of an empty list beside an empty thread.
  if (hasNoGroups && !selectedConversation) {
    return (
      <div className="relative h-full" data-testid="chat-shell">
        <div className="absolute right-3 top-2.5 z-10">{bell}</div>
        <GroupsEmptyState onCreate={() => setNewGroupOpen(true)} disabled={thread.startingThread} />
        <NewGroupDialog open={newGroupOpen} onClose={() => setNewGroupOpen(false)} onNewGroupChat={sendCommand} />
      </div>
    );
  }

  // The list's "+" knows which thread is open, so on a wide screen it offers
  // the same ways in as the thread header's "+" beside it.
  const listColumn = (
    <ConversationList
      scope="groups"
      selectedConversation={selectedConversation}
      onSelectConversation={onSelectConversation}
      compose={
        <>
          {composeMenu(Boolean(selectedConversation))}
          {bell}
        </>
      }
    />
  );

  // The Groups tab with rooms but none open — on a wide screen, the pane
  // beside the list. Starting a one-to-one thread is Home's, so it offers
  // nothing but the rooms on its left.
  const threadPane = !selectedConversation ? (
    <div className="flex flex-1 items-center justify-center px-6" data-testid="groups-pick-one">
      <p className="text-sm text-on-surface-variant">{t('groups.pickOne')}</p>
    </div>
  ) : (
    <div className="flex h-full flex-col">
      <ThreadHeader
        title={thread.title}
        subtitle={thread.subtitle}
        initials={initialsFor(thread.title)}
        avatarSlot={activeConversation ? avatarSlot(activeConversation) : 0}
        avatarShape={activeConversation?.group_id ? 'square' : 'circle'}
        onOpenInfo={() => info.show(false)}
        onBack={isDesktop ? undefined : () => onSelectConversation(null)}
        actions={composeMenu(true)}
      />
      <ChatThread
        thread={thread}
        conversationTitle={thread.title}
        banner={<ThreadConnectBanner thread={thread} onNavigate={onNavigate} />}
        followOnOpen
        focusComposerOnOpen={isDesktop}
      />
    </div>
  );

  return (
    <div className="relative h-full">
      <ChatShell list={listColumn} thread={threadPane} hasSelection={Boolean(selectedConversation)} />
      <ThreadInfoDrawer
        info={info}
        conversation={activeConversation}
        onSendCommand={sendCommand}
        onSelectConversation={onSelectConversation}
        onNavigate={onNavigate}
      />
    </div>
  );
}

/** The Groups tab before the athlete is in any room. */
function GroupsEmptyState({ onCreate, disabled }: { onCreate: () => void; disabled: boolean }) {
  const { t } = useTranslation();
  return (
    <div
      data-testid="groups-empty"
      className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center"
    >
      <span
        aria-hidden="true"
        className="flex h-14 w-14 items-center justify-center rounded-2xl bg-primary-container text-on-primary-container"
      >
        <Users className="h-6 w-6" />
      </span>
      <h2 className="font-display text-lg font-semibold text-on-surface">{t('groups.emptyTitle')}</h2>
      <p className="max-w-[360px] text-sm leading-relaxed text-on-surface-variant">{t('groups.emptyBody')}</p>
      <Button variant="primary" onClick={onCreate} disabled={disabled} className="mt-1" data-testid="groups-empty-create">
        {t('groups.createCta')}
      </Button>
    </div>
  );
}
