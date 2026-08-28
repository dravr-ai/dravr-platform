// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Main dashboard orchestrator with admin sidebar and user mode navigation
// ABOUTME: Admin lands on Users tab; delegates data fetching to focused panel components

import { useState, lazy, Suspense, useEffect, useMemo, useCallback, useRef } from 'react';
import { useAuth } from '../hooks/useAuth';
import { useUnreadCount } from '../hooks/useNotifications';
import { useIsMobile, useIsTablet } from '../hooks/useBreakpoint';
import type { AdminToken } from '../types/api';
import { clsx } from 'clsx';
import { BottomTabBar, MobileDrawer, type MobileNavTab } from './layout/MobileNav';
import { COMMAND_DRAFTS } from '@pierre/shared-constants';
import type { PendingComposerAction } from './ChatTab';
// Explicit /index path avoids macOS case-insensitive collision between
// Dashboard.tsx and dashboard/ directory in Vitest module resolution
import {
  ConversationList,
  usePendingUsersCount,
  useStoreStatsPendingCount,
  useUnreadConversationsCount,
} from './dashboard/index';

// Lazy load heavy components to reduce initial bundle size
/**
 * Tabs only an admin may open.
 *
 * The sidebar already offers a role-appropriate set, but the hash is
 * user-editable and `applyRoute` accepted whatever was typed — so `#users` as a
 * regular user mounted the admin pane. Kept as a literal set rather than derived
 * from the tab arrays because those are built inside the component, after the
 * route handler that needs them.
 */
const ADMIN_ONLY_TABS = new Set([
  'users', 'coaches', 'coach-store', 'configuration', 'user-tools', 'prompts',
  'platform-settings', 'claim-verdicts', 'harness-config', 'guardian-config',
  'memory-worker', 'coach-followups', 'coach-notes-audit', 'myth-busting',
  'coach-grading', 'eval-harness', 'activity', 'engagement', 'connections',
  'analytics', 'admin-tokens', 'billing',
]);

/**
 * Tab ids the product no longer serves.
 *
 * `insights` (the social feed and its friends sub-view) and `my-coaches` (the
 * user Coach tab, folded into Discover) were retired by the Chat-First Cutover
 * on 2026-08-26; `groups` followed when group management moved inside the
 * group's own chat thread. A bookmark, a browser history entry or a
 * notification persisted before those days can still carry the hash. Left to
 * the render switch it would select a tab nothing draws — a blank main pane
 * with a working sidebar — so it resolves to the role's default instead.
 */
const RETIRED_TABS = new Set(['insights', 'my-coaches', 'groups']);

const UsageAnalytics = lazy(() => import('./UsageAnalytics'));
const ActivityTab = lazy(() => import('./ActivityTab'));
const EngagementTab = lazy(() => import('./EngagementTab'));
const UnifiedConnections = lazy(() => import('./UnifiedConnections'));
const UserManagement = lazy(() => import('./UserManagement'));
const UserSettings = lazy(() => import('./UserSettings'));
const AdminSettings = lazy(() => import('./AdminSettings'));
const ApiKeyList = lazy(() => import('./ApiKeyList'));
const ApiKeyDetails = lazy(() => import('./ApiKeyDetails'));
const ChatTab = lazy(() => import('./ChatTab'));
const AdminConfiguration = lazy(() => import('./AdminConfiguration'));
const UserToolOverrides = lazy(() => import('./UserToolOverrides'));
const SystemCoachesTab = lazy(() => import('./SystemCoachesTab'));
const SystemPromptsTab = lazy(() => import('./SystemPromptsTab'));
const ClaimVerdictsTab = lazy(() => import('./ClaimVerdictsTab'));
const HarnessConfigTab = lazy(() => import('./HarnessConfigTab'));
const GuardianConfigTab = lazy(() => import('./GuardianConfigTab'));
const MemoryExtractionMonitorTab = lazy(() => import('./MemoryExtractionMonitorTab'));
const CoachFollowupsTab = lazy(() => import('./CoachFollowupsTab'));
const CoachNotesAuditTab = lazy(() => import('./CoachNotesAuditTab'));
const MythBustingTab = lazy(() => import('./MythBustingTab'));
const CoachGradingTab = lazy(() => import('./CoachGradingTab'));
const EvalHarnessTab = lazy(() => import('./EvalHarnessTab'));
const CoachStoreManagement = lazy(() => import('./CoachStoreManagement'));
const StoreScreen = lazy(() => import('./StoreScreen'));
const LlmConsumptionPanel = lazy(() => import('./LlmConsumptionPanel'));
const ToolUsagePanel = lazy(() => import('./ToolUsagePanel'));
const BillingTab = lazy(() => import('./BillingTab'));
const BillingPage = lazy(() => import('./BillingPage'));
const NotificationsPanel = lazy(() => import('./notifications/NotificationsPanel'));
import { Card } from './ui';
import { ConnectProviderBanner } from './ConnectProviderBanner';
import { BILLING_ENABLED } from '../constants/features';
import { PAGE_GUTTER_CLASS, layoutForRoute } from '../constants/surfaceLayout';
import { useTranslation } from '@pierre/i18n';
import { track } from '../services/analytics';

// Tab definition type with optional badge for notification counts
interface TabDefinition {
  id: string;
  name: string;
  icon: React.ReactNode;
  badge?: number;
  section?: string;
}

import { DravrLogo } from './DravrLogo';

interface DashboardProps {
  pendingInviteCode?: string | null;
  onInviteCodeConsumed?: () => void;
}

export default function Dashboard({ pendingInviteCode, onInviteCodeConsumed }: DashboardProps) {
  const { user, logout } = useAuth();
  const { t } = useTranslation();
  // Default tab depends on user role: admin sees 'users', regular users see 'chat'
  const isAdminUser = user?.role === 'admin' || user?.role === 'super_admin';
  const isSuperAdmin = user?.role === 'super_admin';
  // Initialize from URL hash so deep links (#users, #coaches, …) survive
  // page reloads and bookmarks. Falls back to role default.
  // Route = `tab[/subview]` encoded in the URL hash (e.g. #groups/<id>,
  // #chat/<conversationId>) so sub-views are deep-linkable and the browser /
  // Android hardware Back button pops them.
  const initialRoute = typeof window !== 'undefined' ? window.location.hash.replace(/^#/, '') : '';
  const initialSlash = initialRoute.indexOf('/');
  const initialTabSeg = initialSlash === -1 ? initialRoute : initialRoute.slice(0, initialSlash);
  const initialSubSeg = initialSlash === -1 ? '' : initialRoute.slice(initialSlash + 1);
  // Guard the first paint as well as later hash edits. A hand-typed or
  // bookmarked `#users` arrives as a full page load, which never reaches
  // `applyRoute`, so gating only there would leave the very path a user takes
  // wide open.
  const initialFallback = isAdminUser ? 'users' : 'chat';
  const initialTab =
    (!isAdminUser && ADMIN_ONLY_TABS.has(initialTabSeg)) || RETIRED_TABS.has(initialTabSeg)
      ? initialFallback
      : initialTabSeg || initialFallback;
  const [activeTab, setActiveTab] = useState<string>(initialTab);

  // Hash-route mirroring + back/forward handling live below, after the
  // sub-view state declarations (they reference selectedConversation and
  // editingCoachId, which are declared later in the component).
  // User's manual collapse preference (persisted). The EFFECTIVE collapse
  // (`sidebarCollapsed`, derived below) also forces the rail in the tablet band.
  const [userSidebarCollapsed, setUserSidebarCollapsed] = useState(() => {
    return localStorage.getItem('pierre.sidebar_collapsed') === 'true';
  });
  // 768–1023 (small tablet / phone landscape) forces the 72px icon rail — the
  // full sidebar would eat the narrow viewport. Below 768 the mobile bottom-bar
  // shell hides the sidebar entirely; at ≥1024 the user's preference governs.
  // Because every width/render check below reads `sidebarCollapsed`, deriving it
  // here forces the rail across the whole sidebar (and auto-disables the resize
  // handle, which only renders when `!sidebarCollapsed`).
  const isTablet = useIsTablet();
  const sidebarCollapsed = userSidebarCollapsed || isTablet;
  const showConversationPane = activeTab === 'chat' && !sidebarCollapsed;
  // User-tunable sidebar width when expanded. The default 260px truncates
  // long chat-session titles and the user button's display name (web QA
  // 2026-05-09); a drag handle lets the user widen the panel to fit
  // their content. Bounds keep it useful — narrower than 220 starts
  // truncating again, wider than 480 swallows half the chat pane.
  const SIDEBAR_MIN_WIDTH = 220;
  const SIDEBAR_MAX_WIDTH = 480;
  const SIDEBAR_DEFAULT_WIDTH = 260;
  const [sidebarWidth, setSidebarWidth] = useState<number>(() => {
    const stored = localStorage.getItem('pierre.sidebar_width');
    const parsed = stored ? Number.parseInt(stored, 10) : Number.NaN;
    if (Number.isFinite(parsed) && parsed >= SIDEBAR_MIN_WIDTH && parsed <= SIDEBAR_MAX_WIDTH) {
      return parsed;
    }
    return SIDEBAR_DEFAULT_WIDTH;
  });
  const [isResizingSidebar, setIsResizingSidebar] = useState(false);
  const onSidebarResizeStart = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (sidebarCollapsed) return;
    event.preventDefault();
    setIsResizingSidebar(true);
    const startX = event.clientX;
    const startWidth = sidebarWidth;
    const handleMove = (moveEvent: PointerEvent) => {
      const delta = moveEvent.clientX - startX;
      const next = Math.min(
        SIDEBAR_MAX_WIDTH,
        Math.max(SIDEBAR_MIN_WIDTH, startWidth + delta),
      );
      setSidebarWidth(next);
    };
    const handleUp = () => {
      window.removeEventListener('pointermove', handleMove);
      window.removeEventListener('pointerup', handleUp);
      window.removeEventListener('pointercancel', handleUp);
      setIsResizingSidebar(false);
      // Persist the final width so the next session opens at the user's
      // chosen size; reading the React state here would close over the
      // stale value, so we read the latest via setState's functional form.
      setSidebarWidth(current => {
        localStorage.setItem('pierre.sidebar_width', String(current));
        return current;
      });
    };
    window.addEventListener('pointermove', handleMove);
    window.addEventListener('pointerup', handleUp);
    window.addEventListener('pointercancel', handleUp);
  }, [sidebarCollapsed, sidebarWidth]);
  const [selectedAdminToken, setSelectedAdminToken] = useState<AdminToken | null>(null);
  const [showUserMenu, setShowUserMenu] = useState(false);

  // Use hooks from panel components for badge counts.
  // Gate behind admin role so regular users don't trigger 403s on /api/admin/*.
  const pendingUsersCount = usePendingUsersCount(isAdminUser);
  const storeStatsPendingCount = useStoreStatsPendingCount(isAdminUser);
  const { unreadCount: notificationUnreadCount } = useUnreadCount();
  // Unread chat rows; an operator has no Chat tab, so the list is never fetched for them.
  const unreadConversationsCount = useUnreadConversationsCount(!isAdminUser);

  // Chat conversations state
  const [selectedConversation, setSelectedConversation] = useState<string | null>(
    initialTabSeg === 'chat' && initialSubSeg ? decodeURIComponent(initialSubSeg) : null,
  );

  // The coach whose Discover edit sheet is open, from `#discover/<coachId>`.
  const [editingCoachId, setEditingCoachId] = useState<string | null>(
    initialTabSeg === 'discover' && initialSubSeg ? decodeURIComponent(initialSubSeg) : null,
  );

  // An invite link lands on chat and joins through the command, exactly as a
  // Telegram or WhatsApp member would: one turn, `/group join <code>`, in a
  // thread the athlete can then read.
  const [pendingComposerAction, setPendingComposerAction] = useState<PendingComposerAction | null>(
    null,
  );
  useEffect(() => {
    if (!pendingInviteCode) return;
    setActiveTab('chat');
    setPendingComposerAction({ kind: 'send', text: COMMAND_DRAFTS.groupJoin(pendingInviteCode) });
  }, [pendingInviteCode]);

  // ── URL hash routing ──────────────────────────────────────────────────────
  // Compose the route from the active tab + its open sub-view, so deep links
  // and the Back button operate on sub-views, not just top-level tabs.
  const route = (() => {
    if (activeTab === 'discover' && editingCoachId) return `discover/${encodeURIComponent(editingCoachId)}`;
    if (activeTab === 'chat' && selectedConversation) return `chat/${encodeURIComponent(selectedConversation)}`;
    return activeTab;
  })();

  // Mirror route → location.hash. First sync REPLACES (no spurious back-entry
  // for the initial / bookmarked route); every later change PUSHES so the
  // browser back button and Android hardware Back walk back through visited
  // tabs AND sub-views instead of exiting the app. A back-driven hashchange
  // updates state, this effect re-runs, `current === route` skips the push —
  // no loop, no duplicate entry. The route doubles as the page_view path.
  const hashSyncedOnce = useRef(false);
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const current = window.location.hash.replace(/^#/, '');
    if (current !== route) {
      if (hashSyncedOnce.current) {
        window.history.pushState(null, '', `#${route}`);
      } else {
        window.history.replaceState(null, '', `#${route}`);
      }
    }
    hashSyncedOnce.current = true;
    track({ name: 'page_view', props: { path: `/${route}` } });
  }, [route]);

  // Parse a `tab[/subview]` route and restore both the active tab and its
  // sub-view. An emptied route resets to the role default rather than
  // stranding the previous view. Shared by the hashchange listener
  // (back/forward, external hash edits) and in-app navigators — e.g. the
  // notifications panel, which deep-links a coach "Reply" to
  // `chat/<conversationId>` so the reply opens that thread.
  const applyRoute = useCallback((raw: string) => {
    const slash = raw.indexOf('/');
    const fallback = isAdminUser ? 'users' : 'chat';
    const requested = (slash === -1 ? raw : raw.slice(0, slash)) || fallback;
    // The sidebar only offers tabs for the caller's role, but the hash is
    // user-editable: typing `#users` as a regular user mounted the admin
    // UserManagement pane. The server refuses the data (every /api/admin call
    // 403s), so nothing leaked — but the pane still rendered its filter chrome
    // and then retried the 403 on a loop. Resolve an out-of-role tab back to
    // the role's own default instead of mounting a surface it cannot use.
    const tab =
      (!isAdminUser && ADMIN_ONLY_TABS.has(requested)) || RETIRED_TABS.has(requested)
        ? fallback
        : requested;
    // A rewritten hash typed while the fallback tab is already active changes
    // no state, so the route effect below never runs and the stale hash would
    // stay in the address bar. Replace it here — never push, so Back does not
    // walk into the retired route again.
    if (tab !== requested && typeof window !== 'undefined') {
      window.history.replaceState(null, '', `#${tab}`);
    }
    // A resolved-away tab takes its sub-view with it. `#groups/<groupId>`
    // resolving to chat used to keep the segment and hand the group's id to
    // the chat pane as a conversation id — a thread that does not exist.
    const sub = tab !== requested || slash === -1 ? '' : raw.slice(slash + 1);
    setActiveTab(tab);
    setEditingCoachId(tab === 'discover' && sub ? decodeURIComponent(sub) : null);
    setSelectedConversation(tab === 'chat' && sub ? decodeURIComponent(sub) : null);
  }, [isAdminUser]);

  // React to back/forward and external hash edits.
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const onHashChange = () => applyRoute(window.location.hash.replace(/^#/, ''));
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, [applyRoute]);

  // Close user menu when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (showUserMenu && !(e.target as Element).closest('.user-menu-container')) {
        setShowUserMenu(false);
      }
    };
    document.addEventListener('click', handleClickOutside);
    return () => document.removeEventListener('click', handleClickOutside);
  }, [showUserMenu]);

  // Tab definitions for admin users, grouped into sidebar sections
  const adminTabs: TabDefinition[] = useMemo(() => [
    { id: 'users', name: t('shell.navUsers'), section: 'Platform', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" />
      </svg>
    ), badge: pendingUsersCount > 0 ? pendingUsersCount : undefined },
    { id: 'coaches', name: t('chat.coachesHeading'), section: 'Coaching', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5.121 17.804A13.937 13.937 0 0112 16c2.5 0 4.847.655 6.879 1.804M15 10a3 3 0 11-6 0 3 3 0 016 0zm6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
      </svg>
    ) },
    { id: 'coach-store', name: t('shell.navCoachStore'), section: 'Coaching', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 3h2l.4 2M7 13h10l4-8H5.4M7 13L5.4 5M7 13l-2.293 2.293c-.63.63-.184 1.707.707 1.707H17m0 0a2 2 0 100 4 2 2 0 000-4zm-8 2a2 2 0 11-4 0 2 2 0 014 0z" />
      </svg>
    ), badge: storeStatsPendingCount > 0 ? storeStatsPendingCount : undefined },
    { id: 'configuration', name: t('shell.navToolManagement'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4" />
      </svg>
    ) },
    { id: 'user-tools', name: t('shell.navUserTools'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h9m3-6l2 2 4-4" />
      </svg>
    ) },
    { id: 'prompts', name: t('shell.navPrompts'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
      </svg>
    ) },
    { id: 'platform-settings', name: t('shell.navPlatformSettings'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
      </svg>
    ) },
    { id: 'claim-verdicts', name: t('shell.navClaimVerdicts'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.031 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
      </svg>
    ) },
    { id: 'harness-config', name: t('shell.navHarnessConfig'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M11 4a2 2 0 114 0v1a1 1 0 001 1h3a1 1 0 011 1v3a1 1 0 01-1 1h-1a2 2 0 100 4h1a1 1 0 011 1v3a1 1 0 01-1 1h-3a1 1 0 01-1-1v-1a2 2 0 10-4 0v1a1 1 0 01-1 1H7a1 1 0 01-1-1v-3a1 1 0 00-1-1H4a2 2 0 110-4h1a1 1 0 001-1V7a1 1 0 011-1h3a1 1 0 001-1V4z" />
      </svg>
    ) },
    { id: 'guardian-config', name: t('shell.navGuardianConfig'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 3l7 3v5c0 5.25-3.4 9.74-7 11-3.6-1.26-7-5.75-7-11V6l7-3z" />
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9.5 12l1.8 1.8 3.2-3.6" />
      </svg>
    ) },
    { id: 'memory-worker', name: t('shell.navMemoryWorker'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 7v10a2 2 0 002 2h12a2 2 0 002-2V7M4 7a2 2 0 012-2h12a2 2 0 012 2M4 7l8 6 8-6" />
      </svg>
    ) },
    { id: 'coach-followups', name: t('shell.navCoachFollowups'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
      </svg>
    ) },
    { id: 'coach-notes-audit', name: t('shell.navCoachNotesAudit'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
      </svg>
    ) },
    { id: 'myth-busting', name: t('shell.navMythBusting'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 3l1.664 1.664M21 21l-1.5-1.5m-5.485-1.242L12 17l-3.5-1L9 13.5 7 11l1-2 3 1 2-3.5L15 8l3 1-1.5 2.5L18 14l-3 1 1 3.258m0 0L11 21M3 3l8 8m4 4l4 4M3 3l18 18" />
      </svg>
    ) },
    { id: 'coach-grading', name: t('shell.navCoachGrades'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 19V6l12-3v13M9 19c0 1.105-1.343 2-3 2s-3-.895-3-2 1.343-2 3-2 3 .895 3 2zm12-3c0 1.105-1.343 2-3 2s-3-.895-3-2 1.343-2 3-2 3 .895 3 2zM9 10l12-3" />
      </svg>
    ) },
    { id: 'eval-harness', name: t('shell.navEvalHarness'), section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4" />
      </svg>
    ) },
    { id: 'activity', name: t('auth.activityLabel'), section: 'Platform', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
      </svg>
    ) },
    { id: 'engagement', name: t('shell.navEngagement'), section: 'Platform', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" />
      </svg>
    ) },
    { id: 'notifications', name: t('shell.navNotifications'), section: 'Platform', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9" />
      </svg>
    ), badge: notificationUnreadCount > 0 ? notificationUnreadCount : undefined },
    { id: 'connections', name: t('shell.navServiceTokens'), section: 'Developer', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
      </svg>
    ) },
    { id: 'analytics', name: t('shell.navAnalytics'), section: 'Developer', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
      </svg>
    ) },
    // Billing is gated out of the first release (see constants/features).
    ...(BILLING_ENABLED
      ? [{ id: 'billing', name: t('shell.navBilling'), section: 'Platform', icon: (
        <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 10h18M7 15h2m-2 4h2m4-4h6m-6 4h6M5 5h14a2 2 0 012 2v12a2 2 0 01-2 2H5a2 2 0 01-2-2V7a2 2 0 012-2z" />
        </svg>
      ) }]
      : []),
  ], [pendingUsersCount, storeStatsPendingCount, notificationUnreadCount]);

  // Super admin tabs extend admin tabs with admin token management
  const superAdminTabs: TabDefinition[] = useMemo(() => [
    ...adminTabs,
    { id: 'admin-tokens', name: t('shell.navAdminTokens'), icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
      </svg>
    ) },
  ], [adminTabs]);

  // Regular user tabs - Settings accessible via gear icon, not sidebar
  const regularTabs: TabDefinition[] = useMemo(() => [
    { id: 'chat', name: t('nav.chat'), icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
      </svg>
    ), badge: unreadConversationsCount > 0 ? unreadConversationsCount : undefined },
    // The coach library is a pinned section of Discover, not a tab of its own.
    { id: 'discover', name: t('nav.discover'), icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
      </svg>
    ) },
    { id: 'data-providers', name: t('nav.dataProviders'), icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.141 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
      </svg>
    ) },
    { id: 'notifications', name: t('nav.notifications'), icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9" />
      </svg>
    ), badge: notificationUnreadCount > 0 ? notificationUnreadCount : undefined },
    // Usage renders the billing surface; gated out of the first release.
    ...(BILLING_ENABLED
      ? [{ id: 'usage', name: t('nav.usage'), icon: (
        <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 10h18M7 15h2m-2 4h2m4-4h6m-6 4h6M5 5h14a2 2 0 012 2v12a2 2 0 01-2 2H5a2 2 0 01-2-2V7a2 2 0 012-2z" />
        </svg>
      ) }]
      : []),
  ], [notificationUnreadCount, unreadConversationsCount, t]);

  // For admin users, use sidebar tabs
  const tabs = isSuperAdmin ? superAdminTabs : (isAdminUser ? adminTabs : regularTabs);
  // Whether this surface takes the shell's gutter or paints to the pane edges.
  const pageLayout = layoutForRoute(activeTab);

  // Mobile navigation state — bottom tab bar pins the high-traffic destinations
  // and the rest fall into the off-canvas drawer. Active <768px only.
  const isMobile = useIsMobile();
  const [drawerOpen, setDrawerOpen] = useState(false);
  // For regular users, pin Chat / Discover / Notifications to the bottom bar
  // and route the rest through the drawer. For admin users we use the first
  // three tabs (Users / Coaches / Coach Store) as the primary slots.
  const primaryTabIds = useMemo<string[]>(() => {
    if (isAdminUser) {
      return ['users', 'coaches', 'coach-store'];
    }
    return ['chat', 'discover', 'notifications'];
  }, [isAdminUser, t]);
  const primaryMobileTabs: MobileNavTab[] = useMemo(() => {
    return primaryTabIds
      .map((id) => tabs.find((t) => t.id === id))
      .filter((t): t is TabDefinition => Boolean(t))
      .map((t) => ({ id: t.id, name: t.name, icon: t.icon, badge: t.badge }));
  }, [primaryTabIds, tabs]);
  const secondaryMobileTabs: MobileNavTab[] = useMemo(() => {
    const primary = new Set(primaryTabIds);
    return tabs
      .filter((t) => !primary.has(t.id))
      .map((t) => ({ id: t.id, name: t.name, icon: t.icon, badge: t.badge }));
  }, [primaryTabIds, tabs]);
  const drawerHasBadge = useMemo(
    () => secondaryMobileTabs.some((t) => t.badge !== undefined && t.badge > 0),
    [secondaryMobileTabs],
  );

  // Close the drawer whenever the user picks a tab (selection handles it
  // itself, but also defend against external state changes like deep links).
  useEffect(() => {
    if (!isMobile) setDrawerOpen(false);
  }, [isMobile]);

  // Admin user view: Full sidebar with tabs - Dark Theme
  return (
    <div className="min-h-dvh bg-surface flex">
      {/* Vertical Sidebar - Dark.
          Width animates only when toggling collapse; while the user is
          actively dragging the resize handle we suspend the transition
          so the cursor tracks the edge in real time. */}
      <aside
        className={clsx(
          'hidden md:flex fixed left-0 top-0 h-dvh bg-surface-container-low border-r ghost-border flex-col z-40 overflow-hidden',
          isResizingSidebar ? '' : 'transition-all duration-300 ease-in-out',
        )}
        style={{ width: sidebarCollapsed ? 72 : sidebarWidth }}
      >
        {/* Sidebar accent bar */}
        <div className="absolute top-0 left-0 bottom-0 w-1 bg-gradient-to-b boreal-hero-gradient"></div>

        {/* Logo Section */}
        <div className={clsx(
          'flex items-center border-b ghost-border transition-all duration-300',
          sidebarCollapsed ? 'px-3 py-4 justify-center' : 'px-5 py-5 gap-3'
        )}>
          <DravrLogo />
          {!sidebarCollapsed && (
            <div className="flex flex-col">
              <span className="text-lg font-semibold bg-gradient-to-r boreal-hero-gradient bg-clip-text text-transparent">
                {t('shell.brandName')}
              </span>
            </div>
          )}
        </div>

        {/* Navigation Items. On the Chat tab the nav keeps its own height
            and the conversation pane below takes the rest of the column —
            the list is the sidebar's main content there, not a footnote
            under the tabs. */}
        <nav
          className={clsx(
            'py-4 overflow-x-hidden',
            showConversationPane ? 'flex-shrink-0' : 'flex-1 overflow-y-auto',
          )}
        >
          <ul className="space-y-1 px-3">
            {tabs.map((tab, index) => {
              // Render section header when the section changes
              const prevTab = index > 0 ? tabs[index - 1] : null;
              const showSection = isAdminUser && !sidebarCollapsed && tab.section && tab.section !== prevTab?.section;
              const showDivider = isAdminUser && sidebarCollapsed && tab.section && tab.section !== prevTab?.section && index > 0;

              return (
                <li key={tab.id}>
                  {showSection && (
                    <div className={clsx('px-3 pt-4 pb-1 text-[10px] font-semibold uppercase tracking-wider text-outline', index === 0 && '!pt-0')}>
                      {tab.section}
                    </div>
                  )}
                  {showDivider && (
                    <div className="my-2 border-t ghost-border" />
                  )}
                  <button
                    onClick={() => {
                      setActiveTab(tab.id);
                      // Reset conversation selection when clicking Chat tab to show coach selection
                      if (tab.id === 'chat') {
                        setSelectedConversation(null);
                      }
                    }}
                    className={clsx(
                      'w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium transition-all duration-200 group relative min-h-[44px]',
                      {
                        'bg-gradient-to-r from-primary/20 to-primary-container/10 text-primary shadow-sm': activeTab === tab.id,
                        'text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface': activeTab !== tab.id,
                      },
                      sidebarCollapsed && 'justify-center'
                    )}
                    title={sidebarCollapsed ? tab.name : undefined}
                  >
                    {/* Active indicator */}
                    {activeTab === tab.id && (
                      <div className="absolute left-0 top-1/2 -translate-y-1/2 w-1 h-6 bg-primary rounded-r-full" />
                    )}
                    <div className="relative flex-shrink-0">
                      {tab.icon}
                      {tab.badge && (
                        <span
                          data-testid="pending-users-badge"
                          className="absolute -top-1 -right-1 bg-error text-on-primary text-xs rounded-full h-4 w-4 flex items-center justify-center font-bold text-[10px]"
                        >
                          {tab.badge}
                        </span>
                      )}
                    </div>
                    {!sidebarCollapsed && <span>{tab.name}</span>}
                    {/* Tooltip for collapsed state */}
                    {sidebarCollapsed && (
                      <div className="absolute left-full ml-2 px-2 py-1 bg-surface-container-high backdrop-blur-sm text-on-surface text-xs rounded opacity-0 group-hover:opacity-100 pointer-events-none whitespace-nowrap transition-opacity z-50">
                        {tab.name}
                      </div>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </nav>

        {/* The unified conversation list — every thread the athlete is in,
            whatever created it. Shown while the Chat tab is active and the
            sidebar is wide enough to read a row. */}
        {showConversationPane && (
          <div
            className="flex-1 min-h-0 flex flex-col border-t ghost-border"
            data-testid="conversation-pane"
          >
            <ConversationList
              selectedConversation={selectedConversation}
              onSelectConversation={setSelectedConversation}
            />
          </div>
        )}

        {/* User Profile Section - Bottom of sidebar */}
        <div className={clsx(
          'border-t ghost-border',
          sidebarCollapsed ? 'p-1.5' : 'px-2 py-1.5'
        )}>
          <div className={clsx(
            'flex items-center',
            sidebarCollapsed ? 'flex-col gap-1' : 'gap-2'
          )}>
            {/* Clickable user area - navigates to user Settings */}
            <button
              onClick={() => setActiveTab('settings')}
              className={clsx(
                'flex items-center gap-2 rounded-lg transition-all duration-200 hover:bg-surface-container-low',
                sidebarCollapsed ? 'p-1 flex-col' : 'flex-1 min-w-0 p-1.5'
              )}
              title={t('shell.navOpenSettings')}
              aria-label={t('shell.navOpenSettings')}
            >
              {/* User Avatar with online indicator */}
              <div className="relative flex-shrink-0">
                <div className="w-8 h-8 boreal-hero-gradient rounded-full flex items-center justify-center">
                  <span className="text-xs font-bold text-on-primary">
                    {(user?.display_name || user?.email)?.charAt(0).toUpperCase()}
                  </span>
                </div>
                {/* Online status dot */}
                <div className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 bg-activity rounded-full border-2 border-surface-container-low" />
              </div>

              {!sidebarCollapsed && (
                <div className="flex-1 min-w-0 text-left">
                  <p className="text-[11px] font-medium text-on-surface truncate leading-tight">
                    {user?.display_name || user?.email}
                  </p>
                  <span className="text-[9px] text-on-surface-variant uppercase">
                    {user?.role === 'super_admin' ? t('shell.roleSuperAdmin') : user?.role === 'admin' ? t('shell.roleAdmin') : t('shell.roleUser')}
                  </span>
                </div>
              )}
            </button>

            {/* Settings gear icon - visible shortcut to user settings */}
            <button
              onClick={() => setActiveTab('settings')}
              className={clsx(
                'text-outline hover:text-primary transition-colors flex-shrink-0 flex items-center justify-center',
                sidebarCollapsed ? 'min-w-[44px] min-h-[44px]' : 'min-w-[44px] min-h-[44px]',
                activeTab === 'settings' && 'text-primary'
              )}
              title={t('shell.navSettings')}
              aria-label={t('shell.navSettings')}
            >
              <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
              </svg>
            </button>

            {/* Sign out button */}
            <button
              onClick={logout}
              className="text-outline hover:text-primary transition-colors flex-shrink-0 min-w-[44px] min-h-[44px] flex items-center justify-center"
              title={t('shell.navSignOut')}
              aria-label={t('shell.navSignOut')}
            >
              <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1" />
              </svg>
            </button>
          </div>
        </div>

        {/* Collapse Toggle Button — desktop only; the sidebar itself is
            md:hidden, but this button sits in absolute coords so it would
            still escape the parent if rendered. */}
        <button
          onClick={() => {
            const next = !userSidebarCollapsed;
            localStorage.setItem('pierre.sidebar_collapsed', String(next));
            setUserSidebarCollapsed(next);
          }}
          className="hidden lg:flex absolute -right-5 top-20 w-11 h-11 bg-surface-container-low border ghost-border rounded-full items-center justify-center shadow-sm hover:bg-surface-container hover:border-primary transition-all duration-200 z-[60]"
          title={sidebarCollapsed ? t('shell.sidebarExpand') : t('shell.sidebarCollapse')}
          aria-label={sidebarCollapsed ? t('shell.sidebarExpand') : t('shell.sidebarCollapse')}
        >
          <svg
            className={clsx(
              'w-4 h-4 text-on-surface-variant transition-transform duration-300',
              sidebarCollapsed && 'rotate-180'
            )}
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
            aria-hidden="true"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
          </svg>
        </button>

        {/* Drag handle: a thin invisible strip on the right edge that
            users grab to resize the panel. Only active when expanded;
            the collapse toggle still owns the 72↔width transition. */}
        {!sidebarCollapsed && (
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label={t('shell.sidebarResize')}
            onPointerDown={onSidebarResizeStart}
            className={clsx(
              'absolute top-0 right-0 h-full w-1.5 cursor-col-resize z-50 hover:bg-primary/40 transition-colors',
              isResizingSidebar && 'bg-primary/60',
            )}
          />
        )}
      </aside>

      {/* Main Content Area — margin tracks the sidebar's live width on
          desktop; on mobile the sidebar is hidden so we collapse the gutter
          and let the bottom tab bar own the chrome. */}
      <main
        className={clsx(
          // `min-w-0` is load-bearing: without it, any descendant with an
          // intrinsic content width (long admin tables, wide settings
          // forms) lets <main> blow past the viewport on mobile.
          'flex-1 min-w-0 h-dvh flex flex-col',
          isResizingSidebar ? '' : 'transition-all duration-300 ease-in-out',
        )}
        style={{ marginLeft: isMobile ? 0 : (sidebarCollapsed ? 72 : sidebarWidth) }}
      >
        {/* Top Header Bar - only for admin tabs; user tabs have their own TabHeader */}
        {isAdminUser && (
          <header className="bg-surface-container-low/80 backdrop-blur-lg shadow-sm border-b ghost-border sticky top-0 z-30 flex-shrink-0">
            <div className="px-4 md:px-6 py-3 md:py-4 flex items-center justify-between">
              <div className="min-w-0">
                <h1 className="text-lg md:text-xl font-medium text-on-surface truncate">
                  {tabs.find(t => t.id === activeTab)?.name || (activeTab === 'settings' ? t('shell.navSettings') : '')}
                </h1>
              </div>
            </div>
          </header>
        )}

        {/* Content Area. The gutter is a property of the surface, declared in
            constants/surfaceLayout.ts — never of the viewer's role. It used to
            read `isAdminUser && activeTab !== 'chat'`, which padded every page
            for an operator and left Settings and Data Providers flush
            against the viewport for every regular user.
            `data-page-shell` / `data-page-layout` are what the layout gate in
            e2e/design-sweep.visual.spec.ts measures against. */}
        <div
          data-page-shell=""
          data-page-layout={pageLayout}
          className={clsx(
            'flex-1 overflow-auto',
            pageLayout === 'padded' && PAGE_GUTTER_CLASS,
          )}
          style={{
            // Reserve space for the mobile bottom tab bar (56px) plus the
            // iOS safe-area home indicator. No-op on >=md where the bar is
            // hidden.
            paddingBottom: isMobile ? 'calc(56px + env(safe-area-inset-bottom, 0px))' : undefined,
          }}
        >

          {/* Content */}
          {activeTab === 'discover' && (
            <div className="px-4 pt-4 md:px-6 md:pt-6 empty:hidden">
              <ConnectProviderBanner />
            </div>
          )}
        {/* Overview tab removed — admin lands directly on Users */}

        {activeTab === 'connections' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <UnifiedConnections />
          </Suspense>
        )}
        {activeTab === 'analytics' && (
          <div className="space-y-6">
            <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
              <UsageAnalytics />
            </Suspense>
            <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
              <LlmConsumptionPanel />
            </Suspense>
            <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
              <ToolUsagePanel />
            </Suspense>
          </div>
        )}
        {BILLING_ENABLED && activeTab === 'billing' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <BillingTab />
          </Suspense>
        )}
        {BILLING_ENABLED && activeTab === 'usage' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <BillingPage />
          </Suspense>
        )}
        {activeTab === 'activity' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <ActivityTab />
          </Suspense>
        )}
        {activeTab === 'engagement' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <EngagementTab onNavigate={setActiveTab} />
          </Suspense>
        )}
        {activeTab === 'users' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <UserManagement />
          </Suspense>
        )}
        {activeTab === 'configuration' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <AdminConfiguration />
          </Suspense>
        )}
        {activeTab === 'user-tools' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <UserToolOverrides />
          </Suspense>
        )}
        {activeTab === 'platform-settings' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <AdminSettings />
          </Suspense>
        )}
        {activeTab === 'claim-verdicts' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <ClaimVerdictsTab />
          </Suspense>
        )}
        {activeTab === 'harness-config' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <HarnessConfigTab />
          </Suspense>
        )}
        {activeTab === 'guardian-config' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <GuardianConfigTab />
          </Suspense>
        )}
        {activeTab === 'memory-worker' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <MemoryExtractionMonitorTab />
          </Suspense>
        )}
        {activeTab === 'coach-followups' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <CoachFollowupsTab />
          </Suspense>
        )}
        {activeTab === 'coach-notes-audit' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <CoachNotesAuditTab />
          </Suspense>
        )}
        {activeTab === 'myth-busting' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <MythBustingTab />
          </Suspense>
        )}
        {activeTab === 'coach-grading' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <CoachGradingTab />
          </Suspense>
        )}
        {activeTab === 'eval-harness' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <EvalHarnessTab />
          </Suspense>
        )}
        {activeTab === 'prompts' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <SystemPromptsTab />
          </Suspense>
        )}
        {activeTab === 'coaches' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <SystemCoachesTab />
          </Suspense>
        )}
        {activeTab === 'coach-store' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <CoachStoreManagement />
          </Suspense>
        )}
        {activeTab === 'chat' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <ChatTab
              selectedConversation={selectedConversation}
              onSelectConversation={setSelectedConversation}
              onNavigate={applyRoute}
              pendingComposerAction={pendingComposerAction}
              onPendingComposerActionConsumed={() => {
                setPendingComposerAction(null);
                onInviteCodeConsumed?.();
              }}
            />
          </Suspense>
        )}
        {activeTab === 'discover' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <StoreScreen onNavigate={applyRoute} ownCoachId={editingCoachId} />
          </Suspense>
        )}
        {activeTab === 'settings' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <UserSettings />
          </Suspense>
        )}
        {/* Admins are platform operators: provider connections are a
            user-account surface (mirrors ADMIN_HIDDEN_TABS in UserSettings),
            so the pane is role-gated even against a hand-typed #data-providers. */}
        {activeTab === 'data-providers' && !isAdminUser && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <UserSettings initialTab="connections" hideTabNav />
          </Suspense>
        )}
        {activeTab === 'notifications' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <NotificationsPanel onNavigate={applyRoute} />
          </Suspense>
        )}
        {activeTab === 'admin-tokens' && (
          <div className="space-y-6">
            {selectedAdminToken ? (
              <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
                <ApiKeyDetails
                  token={selectedAdminToken}
                  onBack={() => setSelectedAdminToken(null)}
                  onTokenUpdated={() => setSelectedAdminToken(null)}
                />
              </Suspense>
            ) : (
              <>
                <Card variant="dark">
                  <h2 className="text-xl font-semibold mb-4 text-on-surface">{t('shell.navApiKeyManagement')}</h2>
                  <p className="text-on-surface-variant mb-4">
                    {t('shell.apiKeysDescription')}
                  </p>
                </Card>
                <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
                  <ApiKeyList onViewDetails={setSelectedAdminToken} />
                </Suspense>
              </>
            )}
          </div>
        )}
        </div>
      </main>

      {/* Mobile chrome: bottom tab bar + off-canvas drawer. Gated at the
          React level on `isMobile` so the desktop DOM stays unchanged and
          doesn't ship the drawer's slide-in slab as hidden offscreen
          markup. */}
      {isMobile && (
        <>
          <MobileDrawer
            open={drawerOpen}
            onClose={() => setDrawerOpen(false)}
            secondary={secondaryMobileTabs}
            activeTab={activeTab}
            onSelect={(id) => {
              setActiveTab(id);
              if (id === 'chat') setSelectedConversation(null);
            }}
            userLabel={user?.display_name || user?.email || ''}
            userInitial={(user?.display_name || user?.email)?.charAt(0).toUpperCase() ?? '?'}
            userRole={user?.role === 'super_admin' ? t('shell.roleSuperAdmin') : user?.role === 'admin' ? t('shell.roleAdmin') : t('shell.roleUser')}
            onOpenSettings={() => setActiveTab('settings')}
            onSignOut={logout}
          />
          <BottomTabBar
            primary={primaryMobileTabs}
            activeTab={activeTab}
            onSelect={(id) => {
              setActiveTab(id);
              if (id === 'chat') setSelectedConversation(null);
            }}
            onOpenDrawer={() => setDrawerOpen(true)}
            drawerHasBadge={drawerHasBadge}
          />
        </>
      )}
    </div>
  );
}
