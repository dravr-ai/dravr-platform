// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace's one panel beside the calendar — Historique, the coach's private thread with Dravr, and the athlete's room
// ABOUTME: Text tabs over one body, or a button per tab opening it as a sheet below lg; the threads are the chat surface's own, drawn without a plan card twice or the room's private lane (carnet#1816)

import { useEffect, useRef, useState } from 'react';
import { History, Lock, Users, type LucideIcon } from 'lucide-react';
import { useTranslation, type TFunction } from '@pierre/i18n';
import { GROUP_ROOM_KEYS, PRIVATE_LANE_KEYS } from '@pierre/chat-utils';
import type { CoachRosterAthlete } from '@pierre/shared-types';
import { PLAN_WORKSPACE_KEYS, planHistoryLine } from '@pierre/ui-logic';
import { Button, EmptyState, Tabs, type Tab } from '../../ui';
import EmbeddedChat from '../../chat/EmbeddedChat';
import type { PendingComposerAction, WorkspacePlanSurface } from '../../chat/types';
import { useSubjectPlanHistory } from '../../../hooks/useSessionEditor';
import type { WorkspaceRoom } from './useWorkspaceThreads';

/** The panel's tabs: the plan's history, the coach's private thread with the agent, the athlete's room. */
export type WorkspaceTab = 'history' | 'dravr' | 'group';

/** The id prefix the tabs and the panel body share. */
const PANEL_ID = 'athlete-workspace';

/** What the Groupe tab holds: the athlete's room, why there is none, or the groups' read that failed. */
export type WorkspaceGroupTab =
  | { kind: 'room'; room: WorkspaceRoom }
  | { kind: 'noRoom' }
  | { kind: 'failed'; retry: () => void };

/** The panel's tabs in the reader's words: Historique, the agent by its name, and Groupe when there is one. */
function tabWords(t: TFunction, agentName: string, hasGroup: boolean): { id: WorkspaceTab; label: string }[] {
  return [
    { id: 'history', label: t('home.personal.history') },
    { id: 'dravr', label: agentName },
    ...(hasGroup ? [{ id: 'group' as const, label: t('app.group') }] : []),
  ];
}

const OPENER_ICON: Record<WorkspaceTab, LucideIcon> = { history: History, dravr: Lock, group: Users };

/**
 * The panel's way in below `lg`, where it is a sheet: one button per tab,
 * each opening the sheet on its own tab.
 */
export function WorkspacePanelOpeners({
  agentName,
  hasGroup,
  onOpen,
}: {
  agentName: string;
  hasGroup: boolean;
  onOpen: (tab: WorkspaceTab) => void;
}) {
  const { t } = useTranslation();
  return (
    <nav
      aria-label={t(PLAN_WORKSPACE_KEYS.panel)}
      className="flex shrink-0 items-center gap-1 border-b ghost-border px-2 py-1"
      data-testid="workspace-panel-open"
    >
      {tabWords(t, agentName, hasGroup).map(({ id, label }) => {
        const Icon = OPENER_ICON[id];
        return (
          <button
            key={id}
            type="button"
            aria-haspopup="dialog"
            onClick={() => onOpen(id)}
            data-testid={`workspace-open-${id}`}
            className="inline-flex items-center gap-1.5 rounded-lg px-2.5 text-sm font-medium text-on-surface-variant transition-colors hover:bg-surface-container-low hover:text-on-surface focus-ring touch-target"
          >
            <Icon className="h-4 w-4" aria-hidden="true" />
            {label}
          </button>
        );
      })}
    </nav>
  );
}

interface WorkspacePanelProps {
  tab: WorkspaceTab;
  onTab: (tab: WorkspaceTab) => void;
  athlete: CoachRosterAthlete;
  /** The name the thread's agent goes by. */
  agentName: string;
  /** The coach's plan thread on the athlete; `null` while it is found or opened. */
  planThreadId: string | null;
  /** The Groupe tab, or `null` when the athlete is in no group of the coach's and the tab is absent. */
  group: WorkspaceGroupTab | null;
  /** Open the coach's conversation in the athlete's room. */
  onOpenRoom: () => void;
  roomOpening: boolean;
  roomError: boolean;
  /** « Écrire dans le groupe » for an athlete with no room of their own: the group's room on the Groups tab. */
  onWriteToGroup: () => void;
  /** The plan thread's plan cards lead back to the calendar. */
  planSurface: WorkspacePlanSurface;
  /** Text the workspace hands the plan thread to send — « Rédiger le plan ». */
  composerAction: PendingComposerAction | null;
  onComposerActionConsumed: () => void;
}

/**
 * The workspace decides which conversation each tab draws; a thread never
 * switches it to another from inside, so a request to is not followed.
 */
function keepThread(): void {
  // Nothing to do: the tab's conversation is the workspace's to choose.
}

/** The thread's own empty state while its conversation is found or opened. */
function ThreadLoading() {
  const { t } = useTranslation();
  return (
    <div className="flex flex-1 items-center justify-center py-8" role="status" aria-label={t('common.loading')}>
      <div className="pierre-spinner" />
    </div>
  );
}

/**
 * Historique: what changed in the plan, newest first, each « when · who ·
 * what », a page at a time. « Afficher les changements plus anciens » reads
 * the page before the oldest change shown and adds it below; it goes once a
 * page comes back short. The first change it adds takes focus, so a keyboard
 * reader carries on from where the list grew.
 */
function HistoryTab({ planThreadId, agentName }: { planThreadId: string | null; agentName: string }) {
  const { t, language } = useTranslation();
  const history = useSubjectPlanHistory(planThreadId);
  const events = history.data?.pages.flatMap((page) => page.events);

  // The index of the first change the older page adds, while it is read.
  const listRef = useRef<HTMLOListElement>(null);
  const [focusFrom, setFocusFrom] = useState<number | null>(null);
  const shown = events?.length ?? 0;
  useEffect(() => {
    if (focusFrom === null || shown <= focusFrom) return;
    const first = listRef.current?.children.item(focusFrom);
    if (first instanceof HTMLElement) first.focus();
    setFocusFrom(null);
  }, [focusFrom, shown]);
  const showOlder = () => {
    setFocusFrom(shown);
    void history.fetchNextPage().then((result) => {
      // A page that could not be read adds nothing to take focus.
      if (result.isError) setFocusFrom(null);
    });
  };

  let body: React.ReactNode;
  if (planThreadId === null || (events === undefined && !history.isError)) {
    body = <ThreadLoading />;
  } else if (events === undefined) {
    body = (
      <EmptyState action={{ label: t('common.retry'), onClick: () => void history.refetch() }} data-testid="workspace-history-failed">
        {t(PLAN_WORKSPACE_KEYS.historyFailed)}
      </EmptyState>
    );
  } else if (events.length === 0) {
    body = <EmptyState data-testid="workspace-history-empty">{t(PLAN_WORKSPACE_KEYS.historyEmpty)}</EmptyState>;
  } else {
    body = (
      <>
        <ol ref={listRef} className="m-0 mt-3 flex list-none flex-col gap-3.5 p-0" data-testid="workspace-history">
          {events.map((event) => {
            const line = planHistoryLine(event, { t, language, agentName });
            return (
              <li
                key={`${event.created_at}-${event.kind}-${event.dates.join(',')}`}
                tabIndex={-1}
                className="grid grid-cols-[56px_minmax(0,1fr)] gap-2.5 rounded focus-ring-visible"
                data-testid="workspace-history-event"
              >
                <span className="pt-px font-mono text-xs text-outline">{line.when}</span>
                <span className="flex min-w-0 flex-col">
                  <span className="text-sm font-semibold text-on-surface">{line.who}</span>
                  <span className="text-sm text-on-surface-variant">{line.what}</span>
                </span>
              </li>
            );
          })}
        </ol>
        {history.isFetchNextPageError && (
          <p role="alert" className="m-0 mt-3 text-sm text-error" data-testid="workspace-history-more-failed">
            {t(PLAN_WORKSPACE_KEYS.historyFailed)}
          </p>
        )}
        {history.hasNextPage && (
          <Button
            variant="tertiary"
            size="sm"
            className="mt-3 px-0"
            onClick={showOlder}
            loading={history.isFetchingNextPage}
            data-testid="workspace-history-more"
          >
            {t(PLAN_WORKSPACE_KEYS.historyMore)}
          </Button>
        )}
      </>
    );
  }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
      <p className="m-0 text-sm text-on-surface-variant">{t(PLAN_WORKSPACE_KEYS.historyIntro)}</p>
      {body}
    </div>
  );
}

/**
 * The panel the calendar keeps beside it from `lg`, and the sheet below it.
 * Historique lists the plan's changes. The agent's tab is the coach's plan
 * thread, private to the coach and the agent, its plan cards drawn as one
 * line each since the calendar draws the plan. Groupe is the athlete's own
 * room, the room alone; an athlete whose groups hold other athletes has
 * none, and the tab says so.
 */
export function WorkspacePanel({
  tab,
  onTab,
  athlete,
  agentName,
  planThreadId,
  group,
  onOpenRoom,
  roomOpening,
  roomError,
  onWriteToGroup,
  planSurface,
  composerAction,
  onComposerActionConsumed,
}: WorkspacePanelProps) {
  const { t } = useTranslation();
  const tabs: (Tab & { id: WorkspaceTab })[] = tabWords(t, agentName, group !== null).map((entry) =>
    entry.id === 'dravr' ? { ...entry, icon: <Lock className="h-3 w-3" aria-hidden="true" /> } : entry,
  );
  const active = tabs.some((entry) => entry.id === tab) ? tab : 'history';

  let body: React.ReactNode;
  if (active === 'history') {
    body = <HistoryTab planThreadId={planThreadId} agentName={agentName} />;
  } else if (active === 'dravr') {
    body = (
      <div className="flex min-h-0 flex-1 flex-col">
        <p className="m-0 flex items-center gap-1.5 px-4 pb-1 pt-3 text-xs text-on-surface-variant">
          <Lock className="h-3 w-3 shrink-0" aria-hidden="true" />
          {t(PRIVATE_LANE_KEYS.divider, { agent: agentName })}
        </p>
        <div className="min-h-0 flex-1">
          <EmbeddedChat
            conversationId={planThreadId}
            onSelectConversation={keepThread}
            emptyState={<ThreadLoading />}
            followOnOpen
            planSurface={planSurface}
            pendingComposerAction={composerAction}
            onPendingComposerActionConsumed={onComposerActionConsumed}
          />
        </div>
      </div>
    );
  } else if (group?.kind === 'room') {
    const { room } = group;
    body = (
      <div className="flex min-h-0 flex-1 flex-col">
        <p className="m-0 px-4 pb-1 pt-3 text-xs text-on-surface-variant">{room.group.name}</p>
        <div className="min-h-0 flex-1">
          <EmbeddedChat
            conversationId={room.conversationId}
            onSelectConversation={keepThread}
            followOnOpen
            privateLane={false}
            emptyState={
              <div className="px-4">
                <EmptyState
                  action={{
                    label: roomOpening ? t('coachHome.action.openingGroup') : t('coachHome.action.writeToGroup'),
                    onClick: onOpenRoom,
                    'data-testid': 'workspace-open-room',
                  }}
                  data-testid="workspace-room-unopened"
                >
                  {roomError ? t(GROUP_ROOM_KEYS.openError) : t(GROUP_ROOM_KEYS.empty, { group: room.group.name })}
                </EmptyState>
              </div>
            }
          />
        </div>
      </div>
    );
  } else if (group?.kind === 'failed') {
    body = (
      <div className="px-4 py-3">
        <EmptyState
          action={{ label: t('common.retry'), onClick: group.retry, 'data-testid': 'workspace-room-retry' }}
          data-testid="workspace-room-failed"
        >
          {t(GROUP_ROOM_KEYS.openError)}
        </EmptyState>
      </div>
    );
  } else {
    body = (
      <div className="px-4 py-3">
        <EmptyState
          action={{ label: t('coachHome.action.writeToGroup'), onClick: onWriteToGroup, 'data-testid': 'workspace-write-group' }}
          data-testid="workspace-no-room"
        >
          {t(PLAN_WORKSPACE_KEYS.noRoom, { name: athlete.name })}
        </EmptyState>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="workspace-panel">
      <Tabs
        tabs={tabs}
        activeTab={active}
        onChange={(id) => onTab(tabs.find((entry) => entry.id === id)?.id ?? 'history')}
        idPrefix={PANEL_ID}
        aria-label={t(PLAN_WORKSPACE_KEYS.panel)}
        className="shrink-0 px-4"
      />
      <div
        role="tabpanel"
        id={`${PANEL_ID}-panel-${active}`}
        aria-labelledby={`${PANEL_ID}-tab-${active}`}
        className="flex min-h-0 flex-1 flex-col"
        data-testid={`workspace-panel-${active}`}
      >
        {body}
      </div>
    </div>
  );
}
