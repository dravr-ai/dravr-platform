// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's conversation: the athlete's own thread in the middle, Today beside it, the other threads behind History
// ABOUTME: A new chat opens a topic in the personal thread rather than another thread; the thread itself is ChatThread

import { useCallback, useEffect, useRef, useState, type ReactNode, type UIEvent } from 'react';
import { History, PanelRightClose, PanelRightOpen, Plus } from 'lucide-react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import type { UseCaseStarter } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { chatApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { useIsDesktop, useIsMobile } from '../../hooks/useBreakpoint';
import { useComposerFocused } from '../../hooks/useComposerFocused';
import { ChatComposeMenu, ChatEmptyState } from '../chat';
import ChatThread from '../chat/ChatThread';
import PersonalTopics from '../chat/PersonalTopics';
import ThreadHeader from '../chat/ThreadHeader';
import ThreadConnectBanner from '../chat/ThreadConnectBanner';
import ThreadInfoDrawer from '../chat/ThreadInfoDrawer';
import { useThreadInfo } from '../chat/useThreadInfo';
import { useChatThread, type OpenThread } from '../chat/useChatThread';
import { topicAnchorId } from '../chat/utils';
import ConversationList from '../dashboard/ConversationList';
import { NotificationBell } from '../notifications/NotificationBell';
import { Sheet } from '../ui';
import type { PersonalToday, TodayActions } from './personalToday';

/** Where the docked Today panel's open or closed state is kept, per browser. */
const TODAY_PANEL_STORAGE_KEY = 'dravr.home.todayPanel';

/** How close to the end of the transcript still counts as reading the latest message, in px. */
const LATEST_MESSAGE_SLACK_PX = 64;

function readTodayDocked(): boolean {
  try {
    return localStorage.getItem(TODAY_PANEL_STORAGE_KEY) !== 'closed';
  } catch {
    // Storage can be refused (a private window, blocked site data): the
    // panel then opens, as it does for everyone the first time.
    return true;
  }
}

function writeTodayDocked(open: boolean): void {
  try {
    localStorage.setItem(TODAY_PANEL_STORAGE_KEY, open ? 'open' : 'closed');
  } catch {
    // Nothing to keep it in; the choice holds for this page only.
  }
}

interface PersonalChatProps {
  selectedConversation: string | null;
  onSelectConversation: (id: string | null) => void;
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate?: (route: string) => void;
  /** Home's Today, drawn by the host. */
  today?: PersonalToday;
  /** The host is still working out which conversation Home opens on. */
  resolving?: boolean;
}

export default function PersonalChat({
  selectedConversation,
  onSelectConversation,
  onNavigate,
  today,
  resolving = false,
}: PersonalChatProps) {
  const queryClient = useQueryClient();

  // On Home a new chat is a new topic inside the athlete's personal thread,
  // never another thread: a divider the coach reads afresh from, rebound to
  // the agent they picked. The topic's divider and its agent's welcome are
  // new rows of a thread that may already be on screen, and the first "+" of
  // an athlete with no thread yet is what opened it.
  const openTopic = useCallback<OpenThread>(
    async (request) => {
      const opened = await chatApi.openPersonalTopic(request);
      const id = opened.conversation.id;
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(id) });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.personal() });
      return { id };
    },
    [queryClient],
  );

  const thread = useChatThread({
    conversationId: selectedConversation,
    onSelectConversation,
    openThread: openTopic,
  });
  const { history, runComposerAction, startThread, activeConversation } = thread;
  const info = useThreadInfo(selectedConversation);

  // Home names the thread after the agent bound to it, in the athlete's
  // language — the same name its replies are signed with. The stored title
  // is the server's, stamped with the agent's canonical (English) title when
  // a topic rebinds the thread, so printing it put "Fuelling Agent" over
  // replies signed "Agent Ravitaillement" (carnet#1146). A rename shows in
  // the History list; the header follows the agent.
  const title = thread.activeCoachTitle ?? thread.title;

  const isDesktop = useIsDesktop();
  const isMobile = useIsMobile();
  const composing = useComposerFocused();

  // Home's sheets and its docked Today panel. The panel's open state is the
  // athlete's choice and outlives the page; the sheets close with it.
  const [historyOpen, setHistoryOpen] = useState(false);
  // A topic picked in History: the thread opens where it starts, reading
  // windows back until its divider is held (carnet#840).
  const [pendingTopic, setPendingTopic] = useState<{ conversationId: string; dividerId: string | null } | null>(null);
  const [todaySheetOpen, setTodaySheetOpen] = useState(false);
  const [todayDocked, setTodayDocked] = useState(readTodayDocked);
  // Whether the transcript is scrolled to its latest message. Home's folded
  // Today line steps aside while the athlete reads back up the thread and
  // returns once they are at the end of it again (Phil, 2026-10-05).
  const [atLatest, setAtLatest] = useState(true);
  // Where the transcript was at its last scroll event, for the direction.
  const lastScrollTopRef = useRef(0);

  // A new thread is read from its latest message.
  useEffect(() => {
    setAtLatest(true);
    lastScrollTopRef.current = 0;
  }, [selectedConversation]);

  // Open a picked topic: read back until its divider — or, for the thread's
  // opening, its first row — is held, then bring it into view.
  const { loaded, ownMessages, hasEarlier, heldRows, earlierLoading, loadBefore } = history;
  useEffect(() => {
    if (!pendingTopic || pendingTopic.conversationId !== selectedConversation || !loaded) return;
    const { dividerId } = pendingTopic;
    const held = dividerId === null ? !hasEarlier : (ownMessages ?? []).some(m => m.id === dividerId);
    if (held) {
      setPendingTopic(null);
      requestAnimationFrame(() => {
        const target = dividerId
          ? document.getElementById(topicAnchorId(dividerId))
          : document.querySelector('[data-testid="message-row"]');
        target?.scrollIntoView({ block: 'start' });
      });
      return;
    }
    if (!hasEarlier || heldRows.length === 0) {
      setPendingTopic(null);
      return;
    }
    if (!earlierLoading) void loadBefore(heldRows);
  }, [pendingTopic, selectedConversation, loaded, ownMessages, hasEarlier, heldRows, earlierLoading, loadBefore]);

  const openTopicAt = useCallback(
    (conversationId: string, dividerId: string | null) => {
      setPendingTopic({ conversationId, dividerId });
      onSelectConversation(conversationId);
    },
    [onSelectConversation],
  );

  /**
   * The starters the empty thread offers, ranked from the athlete's state.
   * Read each time the empty thread is drawn — the server counts each read
   * as one showing — and never while Home is still choosing its thread (a
   * spinner stands there, and the thread it lands on is usually not empty),
   * on a focus or on a reconnect, which would count showings nobody looked
   * at.
   */
  const emptyThread = !selectedConversation && !resolving;
  const { data: starters } = useQuery<UseCaseStarter[]>({
    queryKey: QUERY_KEYS.chat.starters(),
    queryFn: () => chatApi.listStarters(),
    enabled: emptyThread,
    staleTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: false,
  });

  /**
   * A starter tapped in the empty thread: a fresh thread whose first line is
   * the starter's postback, its label on the athlete's bubble. The athlete
   * speaks first, so the agent posts no welcome offering the same starters
   * over the answer (carnet#829).
   */
  const handleStarter = useCallback((starter: UseCaseStarter) => {
    runComposerAction({ kind: 'send', text: starter.postback, display: starter.label });
  }, [runComposerAction]);

  /**
   * Home's folded Today line follows the athlete's reading: it steps aside
   * when they scroll up the thread and comes back once they reach its latest
   * message. Only a move up hides it — the thread's own scroll to its end on
   * open and on every reply runs downward and may stop short of the last
   * pixel, so it never folds the line away by itself.
   */
  const handleTranscriptScroll = useCallback((event: UIEvent<HTMLDivElement>) => {
    const scroller = event.currentTarget;
    const previous = lastScrollTopRef.current;
    lastScrollTopRef.current = scroller.scrollTop;
    if (scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= LATEST_MESSAGE_SLACK_PX) {
      setAtLatest(true);
    } else if (scroller.scrollTop < previous) {
      setAtLatest(false);
    }
  }, []);

  const sendCommand = useCallback(
    (command: string) => runComposerAction({ kind: 'send', text: command }),
    [runComposerAction],
  );

  /** The "+" menu, wired the same way in every header slot. */
  const composeMenu = (withParticipants: boolean) => (
    <ChatComposeMenu
      onNewChat={() => startThread()}
      onNewGroupChat={sendCommand}
      onAddParticipant={withParticipants ? () => info.show(true) : undefined}
      disabled={thread.startingThread}
    />
  );

  return (
    <PersonalSurface
      selectedConversation={selectedConversation}
      resolving={resolving}
      isDesktop={isDesktop}
      isMobile={isMobile}
      today={today}
      todayDocked={todayDocked}
      onToggleTodayDocked={() => {
        setTodayDocked((open) => {
          writeTodayDocked(!open);
          return !open;
        });
      }}
      todaySheetOpen={todaySheetOpen}
      onTodaySheetOpenChange={setTodaySheetOpen}
      historyOpen={historyOpen}
      onHistoryOpenChange={setHistoryOpen}
      bell={<NotificationBell onNavigate={onNavigate} />}
      peekHidden={!atLatest || composing}
      todayActions={{
        draft: (text) => {
          setTodaySheetOpen(false);
          runComposerAction({ kind: 'draft', text });
        },
        send: (text, display) => {
          setTodaySheetOpen(false);
          runComposerAction({ kind: 'send', text, display });
        },
        openTopic: (conversationId, dividerId) => {
          setTodaySheetOpen(false);
          openTopicAt(conversationId, dividerId);
        },
        dismiss: () => setTodaySheetOpen(false),
      }}
      onSelectConversation={onSelectConversation}
      onOpenTopic={openTopicAt}
      onNewConversation={() => startThread()}
      creating={thread.startingThread}
      renderHeader={(lead, trail) =>
        selectedConversation ? (
          <ThreadHeader
            title={title}
            subtitle={thread.subtitle}
            initials={initialsFor(title)}
            avatarSlot={activeConversation ? avatarSlot(activeConversation) : 0}
            avatarShape={activeConversation?.group_id ? 'square' : 'circle'}
            onOpenInfo={() => info.show(false)}
            actions={
              <>
                {lead}
                {composeMenu(true)}
                {trail}
              </>
            }
          />
        ) : null
      }
      thread={
        <ChatThread
          thread={thread}
          conversationTitle={title}
          banner={<ThreadConnectBanner thread={thread} onNavigate={onNavigate} />}
          followOnOpen
          focusComposerOnOpen={isDesktop}
          onTranscriptScroll={handleTranscriptScroll}
        />
      }
      emptyState={
        <ChatEmptyState
          compose={composeMenu(false)}
          starters={starters ?? []}
          onStarter={handleStarter}
          disabled={thread.startingThread}
          providerStatus={thread.providerStatus}
        />
      }
      drawers={
        <ThreadInfoDrawer
          info={info}
          conversation={activeConversation}
          onSendCommand={sendCommand}
          onSelectConversation={onSelectConversation}
          onNavigate={onNavigate}
        />
      }
    />
  );
}

interface PersonalSurfaceProps {
  selectedConversation: string | null;
  resolving: boolean;
  isDesktop: boolean;
  isMobile: boolean;
  today: PersonalToday | undefined;
  todayDocked: boolean;
  onToggleTodayDocked: () => void;
  todaySheetOpen: boolean;
  onTodaySheetOpenChange: (open: boolean) => void;
  historyOpen: boolean;
  onHistoryOpenChange: (open: boolean) => void;
  /** The notifications bell, the right-most control of the header. */
  bell: ReactNode;
  peekHidden: boolean;
  /** What Today's sections do to this thread. */
  todayActions: TodayActions;
  onSelectConversation: (id: string | null) => void;
  /** Open the personal thread where one of its topics starts. */
  onOpenTopic: (conversationId: string, dividerId: string | null) => void;
  onNewConversation: () => void;
  creating: boolean;
  /**
   * The open thread's header, given Home's own buttons to place around the
   * "+": the history first, the Today panel's toggle last. `null` before
   * there is a thread.
   */
  renderHeader: (lead: ReactNode, trail: ReactNode) => ReactNode;
  /** The open thread's banners, transcript and composer. */
  thread: ReactNode;
  /** What Home shows when the athlete has no conversation yet. */
  emptyState: ReactNode;
  drawers: ReactNode;
}

/**
 * Home: the athlete's own conversation in the middle, Today beside it.
 *
 * At ≥1024px Today is a docked panel the athlete can fold away; below it the
 * panel is one line above the thread that opens a drawer (tablet) or a
 * full-height sheet (phone). The other personal threads — plain, agents,
 * Telegram and the other channels — sit behind the history button.
 */
function PersonalSurface({
  selectedConversation,
  resolving,
  isDesktop,
  isMobile,
  today,
  todayDocked,
  onToggleTodayDocked,
  todaySheetOpen,
  onTodaySheetOpenChange,
  historyOpen,
  onHistoryOpenChange,
  bell,
  peekHidden,
  todayActions,
  onSelectConversation,
  onOpenTopic,
  onNewConversation,
  creating,
  renderHeader,
  thread,
  emptyState,
  drawers,
}: PersonalSurfaceProps) {
  const { t } = useTranslation();
  const iconButton =
    'flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-lg px-2 text-sm font-medium text-on-surface-variant transition-colors hover:bg-surface-container-low hover:text-on-surface focus-ring touch-target';

  const historyButton = (
    <button
      type="button"
      onClick={() => onHistoryOpenChange(true)}
      data-testid="home-history-button"
      aria-haspopup="dialog"
      title={t('home.personal.history')}
      className={iconButton}
    >
      <History className="h-[18px] w-[18px]" aria-hidden="true" />
      <span className="hidden md:inline">{t('home.personal.history')}</span>
      <span className="sr-only md:hidden">{t('home.personal.history')}</span>
    </button>
  );
  const todayToggle =
    isDesktop && today ? (
      <button
        type="button"
        onClick={onToggleTodayDocked}
        data-testid="home-today-toggle"
        aria-pressed={todayDocked}
        aria-label={todayDocked ? t('home.personal.todayHide') : t('home.personal.todayShow')}
        title={todayDocked ? t('home.personal.todayHide') : t('home.personal.todayShow')}
        className={iconButton}
      >
        {todayDocked ? (
          <PanelRightClose className="h-[18px] w-[18px]" aria-hidden="true" />
        ) : (
          <PanelRightOpen className="h-[18px] w-[18px]" aria-hidden="true" />
        )}
      </button>
    ) : null;

  const peek = !isDesktop && today ? today.peek(() => onTodaySheetOpenChange(true), peekHidden) : null;

  let centre: ReactNode;
  if (selectedConversation) {
    centre = (
      <>
        {renderHeader(
          historyButton,
          <>
            {todayToggle}
            {bell}
          </>,
        )}
        {peek}
        {thread}
      </>
    );
  } else {
    centre = (
      <>
        <div className="flex h-[52px] shrink-0 items-center justify-between gap-2 border-b ghost-border bg-surface pl-4 pr-2 md:pl-5 md:pr-3">
          <h2 className="font-display text-lg font-semibold text-on-surface">{t('nav.home')}</h2>
          <div className="flex shrink-0 items-center gap-0.5">
            {historyButton}
            {todayToggle}
            {bell}
          </div>
        </div>
        {peek}
        {resolving ? (
          <div className="flex flex-1 items-center justify-center" role="status" aria-label={t('common.loading')}>
            <div className="pierre-spinner" />
          </div>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden">{emptyState}</div>
        )}
      </>
    );
  }

  return (
    <div className="relative flex h-full min-h-0 bg-surface" data-testid="home-page">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">{centre}</div>

      {isDesktop && today && todayDocked && (
        <aside
          aria-label={t('chat.dayToday')}
          data-testid="home-today-panel"
          className="flex w-[320px] shrink-0 flex-col border-l ghost-border bg-surface xl:w-[360px]"
        >
          <div className="min-h-0 flex-1 overflow-y-auto">{today.panel(todayActions, true)}</div>
        </aside>
      )}

      {!isDesktop && today && todaySheetOpen && (
        <Sheet
          side={isMobile ? 'bottom' : 'right'}
          title={t('chat.dayToday')}
          onClose={() => onTodaySheetOpenChange(false)}
          data-testid="home-today-sheet"
        >
          {today.panel(todayActions, false)}
        </Sheet>
      )}

      {historyOpen && (
        <Sheet
          side="left"
          width="full"
          title={t('home.personal.history')}
          onClose={() => onHistoryOpenChange(false)}
          data-testid="home-history"
          actions={
            <button
              type="button"
              onClick={() => {
                onHistoryOpenChange(false);
                onNewConversation();
              }}
              disabled={creating}
              aria-label={t('chat.newChat')}
              title={t('chat.newChat')}
              data-testid="home-history-new"
              className="flex h-11 w-11 items-center justify-center rounded-lg text-primary transition-colors hover:bg-surface-container-low disabled:opacity-50 focus-ring"
            >
              <Plus className="h-5 w-5" aria-hidden="true" />
            </button>
          }
        >
          <div className="flex h-full min-h-0 flex-col">
            <PersonalTopics
              onOpenTopic={(conversationId, dividerId) => {
                onHistoryOpenChange(false);
                onOpenTopic(conversationId, dividerId);
              }}
            />
            <div className="min-h-0 flex-1">
              <ConversationList
                scope="personal"
                showTitle={false}
                selectedConversation={selectedConversation}
                onSelectConversation={(id) => {
                  onHistoryOpenChange(false);
                  onSelectConversation(id);
                }}
              />
            </div>
          </div>
        </Sheet>
      )}

      {drawers}
    </div>
  );
}
