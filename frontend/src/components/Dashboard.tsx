// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Main dashboard orchestrator with admin sidebar and user mode navigation
// ABOUTME: Admin lands on Users tab; delegates data fetching to focused panel components

import { useState, lazy, Suspense, useEffect, useMemo } from 'react';
import { useAuth } from '../hooks/useAuth';
import { useUnreadCount } from '../hooks/useNotifications';
import type { AdminToken } from '../types/api';
import { clsx } from 'clsx';
// Explicit /index path avoids macOS case-insensitive collision between
// Dashboard.tsx and dashboard/ directory in Vitest module resolution
import {
  ConversationsPanel,
  usePendingUsersCount,
  useStoreStatsPendingCount,
} from './dashboard/index';

// Lazy load heavy components to reduce initial bundle size
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
const SystemCoachesTab = lazy(() => import('./SystemCoachesTab'));
const SystemPromptsTab = lazy(() => import('./SystemPromptsTab'));
const ClaimVerdictsTab = lazy(() => import('./ClaimVerdictsTab'));
const HarnessConfigTab = lazy(() => import('./HarnessConfigTab'));
const MemoryExtractionMonitorTab = lazy(() => import('./MemoryExtractionMonitorTab'));
const CoachFollowupsTab = lazy(() => import('./CoachFollowupsTab'));
const CoachNotesAuditTab = lazy(() => import('./CoachNotesAuditTab'));
const MythBustingTab = lazy(() => import('./MythBustingTab'));
const CoachGradingTab = lazy(() => import('./CoachGradingTab'));
const EvalHarnessTab = lazy(() => import('./EvalHarnessTab'));
const CoachStoreManagement = lazy(() => import('./CoachStoreManagement'));
const CoachLibraryTab = lazy(() => import('./CoachLibraryTab'));
const StoreScreen = lazy(() => import('./StoreScreen'));
const FriendsTab = lazy(() => import('./social/FriendsTab'));
const SocialFeedTab = lazy(() => import('./social/SocialFeedTab'));
const LlmConsumptionPanel = lazy(() => import('./LlmConsumptionPanel'));
const NotificationsPanel = lazy(() => import('./notifications/NotificationsPanel'));
const GroupManagement = lazy(() => import('./groups/GroupManagement'));
const GroupDetail = lazy(() => import('./groups/GroupDetail'));
import { NotificationBell } from './notifications/NotificationBell';
import { Card } from './ui';

// Tab definition type with optional badge for notification counts
interface TabDefinition {
  id: string;
  name: string;
  icon: React.ReactNode;
  badge?: number;
  section?: string;
}

const PierreLogo = () => (
  <svg width="48" height="48" viewBox="0 0 120 120" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
    <defs>
      <linearGradient id="pg" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stopColor="#8B5CF6"/><stop offset="100%" stopColor="#22D3EE"/></linearGradient>
      <linearGradient id="ag" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stopColor="#4ADE80"/><stop offset="100%" stopColor="#22C55E"/></linearGradient>
      <linearGradient id="ng" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stopColor="#F59E0B"/><stop offset="100%" stopColor="#D97706"/></linearGradient>
      <linearGradient id="rg" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stopColor="#818CF8"/><stop offset="100%" stopColor="#6366F1"/></linearGradient>
    </defs>
    <g strokeWidth="2" opacity="0.5" strokeLinecap="round">
      <line x1="40" y1="30" x2="52" y2="42" stroke="url(#ag)"/><line x1="52" y1="42" x2="70" y2="35" stroke="url(#ag)"/>
      <line x1="52" y1="42" x2="48" y2="55" stroke="url(#pg)"/><line x1="48" y1="55" x2="75" y2="52" stroke="url(#ng)"/>
      <line x1="48" y1="55" x2="55" y2="72" stroke="url(#pg)"/><line x1="55" y1="72" x2="35" y2="85" stroke="url(#rg)"/><line x1="55" y1="72" x2="72" y2="82" stroke="url(#rg)"/>
    </g>
    <circle cx="40" cy="30" r="7" fill="url(#ag)"/><circle cx="52" cy="42" r="5" fill="url(#ag)"/><circle cx="70" cy="35" r="3.5" fill="url(#ag)"/>
    <circle cx="48" cy="55" r="6" fill="url(#pg)"/><circle cx="48" cy="55" r="3" fill="#fff" opacity="0.9"/>
    <circle cx="75" cy="52" r="4.5" fill="url(#ng)"/><circle cx="88" cy="60" r="3.5" fill="url(#ng)"/>
    <circle cx="55" cy="72" r="5" fill="url(#rg)"/><circle cx="35" cy="85" r="4" fill="url(#rg)"/><circle cx="72" cy="82" r="4" fill="url(#rg)"/>
  </svg>
);

interface DashboardProps {
  pendingInviteCode?: string | null;
  onInviteCodeConsumed?: () => void;
}

export default function Dashboard({ pendingInviteCode, onInviteCodeConsumed }: DashboardProps) {
  const { user, logout } = useAuth();
  // Default tab depends on user role: admin sees 'users', regular users see 'chat'
  const isAdminUser = user?.role === 'admin' || user?.role === 'super_admin';
  const isSuperAdmin = user?.role === 'super_admin';
  const [activeTab, setActiveTab] = useState(isAdminUser ? 'users' : 'chat');
  // Sub-view state for insights tab (feed vs friends), matching mobile's social stack
  const [insightsView, setInsightsView] = useState<'feed' | 'friends'>('feed');
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    return localStorage.getItem('pierre.sidebar_collapsed') === 'true';
  });
  const [selectedAdminToken, setSelectedAdminToken] = useState<AdminToken | null>(null);
  const [showUserMenu, setShowUserMenu] = useState(false);

  // Use hooks from panel components for badge counts
  const pendingUsersCount = usePendingUsersCount();
  const storeStatsPendingCount = useStoreStatsPendingCount();
  const { unreadCount: notificationUnreadCount } = useUnreadCount();

  // Chat conversations state
  const [selectedConversation, setSelectedConversation] = useState<string | null>(null);

  // Groups state — null shows list, a group ID shows detail
  const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);

  // Auto-navigate to Groups tab when an invite link is detected
  useEffect(() => {
    if (pendingInviteCode) {
      setActiveTab('groups');
      setSelectedGroupId(null);
    }
  }, [pendingInviteCode]);

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
    { id: 'users', name: 'Users', section: 'Platform', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" />
      </svg>
    ), badge: pendingUsersCount > 0 ? pendingUsersCount : undefined },
    { id: 'coaches', name: 'Coaches', section: 'Coaching', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5.121 17.804A13.937 13.937 0 0112 16c2.5 0 4.847.655 6.879 1.804M15 10a3 3 0 11-6 0 3 3 0 016 0zm6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
      </svg>
    ) },
    { id: 'coach-store', name: 'Coach Store', section: 'Coaching', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 3h2l.4 2M7 13h10l4-8H5.4M7 13L5.4 5M7 13l-2.293 2.293c-.63.63-.184 1.707.707 1.707H17m0 0a2 2 0 100 4 2 2 0 000-4zm-8 2a2 2 0 11-4 0 2 2 0 014 0z" />
      </svg>
    ), badge: storeStatsPendingCount > 0 ? storeStatsPendingCount : undefined },
    { id: 'groups', name: 'Groups', section: 'Coaching', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z" />
      </svg>
    ) },
    { id: 'configuration', name: 'Tool Management', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4" />
      </svg>
    ) },
    { id: 'prompts', name: 'Prompts', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
      </svg>
    ) },
    { id: 'platform-settings', name: 'Platform Settings', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
      </svg>
    ) },
    { id: 'claim-verdicts', name: 'Claim Verdicts', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.031 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
      </svg>
    ) },
    { id: 'harness-config', name: 'Harness Config', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M11 4a2 2 0 114 0v1a1 1 0 001 1h3a1 1 0 011 1v3a1 1 0 01-1 1h-1a2 2 0 100 4h1a1 1 0 011 1v3a1 1 0 01-1 1h-3a1 1 0 01-1-1v-1a2 2 0 10-4 0v1a1 1 0 01-1 1H7a1 1 0 01-1-1v-3a1 1 0 00-1-1H4a2 2 0 110-4h1a1 1 0 001-1V7a1 1 0 011-1h3a1 1 0 001-1V4z" />
      </svg>
    ) },
    { id: 'memory-worker', name: 'Memory Worker', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 7v10a2 2 0 002 2h12a2 2 0 002-2V7M4 7a2 2 0 012-2h12a2 2 0 012 2M4 7l8 6 8-6" />
      </svg>
    ) },
    { id: 'coach-followups', name: 'Coach Followups', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
      </svg>
    ) },
    { id: 'coach-notes-audit', name: 'Coach Notes Audit', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
      </svg>
    ) },
    { id: 'myth-busting', name: 'Myth Busting', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 3l1.664 1.664M21 21l-1.5-1.5m-5.485-1.242L12 17l-3.5-1L9 13.5 7 11l1-2 3 1 2-3.5L15 8l3 1-1.5 2.5L18 14l-3 1 1 3.258m0 0L11 21M3 3l8 8m4 4l4 4M3 3l18 18" />
      </svg>
    ) },
    { id: 'coach-grading', name: 'Coach Grades', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 19V6l12-3v13M9 19c0 1.105-1.343 2-3 2s-3-.895-3-2 1.343-2 3-2 3 .895 3 2zm12-3c0 1.105-1.343 2-3 2s-3-.895-3-2 1.343-2 3-2 3 .895 3 2zM9 10l12-3" />
      </svg>
    ) },
    { id: 'eval-harness', name: 'Eval Harness', section: 'Configuration', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4" />
      </svg>
    ) },
    { id: 'activity', name: 'Activity', section: 'Platform', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
      </svg>
    ) },
    { id: 'engagement', name: 'Engagement', section: 'Platform', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" />
      </svg>
    ) },
    { id: 'notifications', name: 'Notifications', section: 'Platform', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9" />
      </svg>
    ), badge: notificationUnreadCount > 0 ? notificationUnreadCount : undefined },
    { id: 'connections', name: 'Service Tokens', section: 'Developer', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
      </svg>
    ) },
    { id: 'analytics', name: 'Analytics', section: 'Developer', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
      </svg>
    ) },
  ], [pendingUsersCount, storeStatsPendingCount, notificationUnreadCount]);

  // Super admin tabs extend admin tabs with admin token management
  const superAdminTabs: TabDefinition[] = useMemo(() => [
    ...adminTabs,
    { id: 'admin-tokens', name: 'Admin Tokens', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
      </svg>
    ) },
  ], [adminTabs]);

  // Regular user tabs - Settings accessible via gear icon, not sidebar
  const regularTabs: TabDefinition[] = useMemo(() => [
    { id: 'chat', name: 'Chat', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
      </svg>
    ) },
    { id: 'my-coaches', name: 'Coaches', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" />
      </svg>
    ) },
    { id: 'discover', name: 'Discover', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
      </svg>
    ) },
    { id: 'groups', name: 'Groups', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z" />
      </svg>
    ) },
    { id: 'insights', name: 'Insights', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 20H5a2 2 0 01-2-2V6a2 2 0 012-2h10a2 2 0 012 2v1m2 13a2 2 0 01-2-2V7m2 13a2 2 0 002-2V9a2 2 0 00-2-2h-2m-4-3H9M7 16h6M7 8h6v4H7V8z" />
      </svg>
    ) },
    { id: 'notifications', name: 'Notifications', icon: (
      <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9" />
      </svg>
    ), badge: notificationUnreadCount > 0 ? notificationUnreadCount : undefined },
  ], [notificationUnreadCount]);

  // For admin users, use sidebar tabs
  const tabs = isSuperAdmin ? superAdminTabs : (isAdminUser ? adminTabs : regularTabs);

  // Admin user view: Full sidebar with tabs - Dark Theme
  return (
    <div className="min-h-screen bg-pierre-dark flex">
      {/* Vertical Sidebar - Dark */}
      <aside
        className={clsx(
          'fixed left-0 top-0 h-screen bg-pierre-slate border-r border-white/10 flex flex-col z-40 transition-all duration-300 ease-in-out overflow-hidden',
          sidebarCollapsed ? 'w-[72px]' : 'w-[260px]'
        )}
      >
        {/* Sidebar accent bar */}
        <div className="absolute top-0 left-0 bottom-0 w-1 bg-gradient-to-b from-pierre-violet via-pierre-cyan to-pierre-activity"></div>

        {/* Logo Section */}
        <div className={clsx(
          'flex items-center border-b border-white/10 transition-all duration-300',
          sidebarCollapsed ? 'px-3 py-4 justify-center' : 'px-5 py-5 gap-3'
        )}>
          <PierreLogo />
          {!sidebarCollapsed && (
            <div className="flex flex-col">
              <span className="text-lg font-semibold bg-gradient-to-r from-pierre-violet to-pierre-cyan bg-clip-text text-transparent">
                Dravr
              </span>
            </div>
          )}
        </div>

        {/* Navigation Items */}
        <nav className="flex-1 py-4 overflow-y-auto overflow-x-hidden">
          <ul className="space-y-1 px-3">
            {tabs.map((tab, index) => {
              // Render section header when the section changes
              const prevTab = index > 0 ? tabs[index - 1] : null;
              const showSection = isAdminUser && !sidebarCollapsed && tab.section && tab.section !== prevTab?.section;
              const showDivider = isAdminUser && sidebarCollapsed && tab.section && tab.section !== prevTab?.section && index > 0;

              return (
                <li key={tab.id}>
                  {showSection && (
                    <div className={clsx('px-3 pt-4 pb-1 text-[10px] font-semibold uppercase tracking-wider text-zinc-500', index === 0 && '!pt-0')}>
                      {tab.section}
                    </div>
                  )}
                  {showDivider && (
                    <div className="my-2 border-t border-white/10" />
                  )}
                  <button
                    onClick={() => {
                      setActiveTab(tab.id);
                      // Reset conversation selection when clicking Chat tab to show coach selection
                      if (tab.id === 'chat') {
                        setSelectedConversation(null);
                      }
                      // Reset insights sub-view when navigating back to insights
                      if (tab.id === 'insights') {
                        setInsightsView('feed');
                      }
                    }}
                    className={clsx(
                      'w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium transition-all duration-200 group relative min-h-[44px]',
                      {
                        'bg-gradient-to-r from-pierre-violet/20 to-pierre-cyan/10 text-pierre-violet-light shadow-sm': activeTab === tab.id,
                        'text-zinc-400 hover:bg-white/5 hover:text-white': activeTab !== tab.id,
                      },
                      sidebarCollapsed && 'justify-center'
                    )}
                    title={sidebarCollapsed ? tab.name : undefined}
                  >
                    {/* Active indicator */}
                    {activeTab === tab.id && (
                      <div className="absolute left-0 top-1/2 -translate-y-1/2 w-1 h-6 bg-pierre-violet rounded-r-full" />
                    )}
                    <div className="relative flex-shrink-0">
                      {tab.icon}
                      {tab.badge && (
                        <span
                          data-testid="pending-users-badge"
                          className="absolute -top-1 -right-1 bg-pierre-red-500 text-white text-xs rounded-full h-4 w-4 flex items-center justify-center font-bold text-[10px]"
                        >
                          {tab.badge}
                        </span>
                      )}
                    </div>
                    {!sidebarCollapsed && <span>{tab.name}</span>}
                    {/* Tooltip for collapsed state */}
                    {sidebarCollapsed && (
                      <div className="absolute left-full ml-2 px-2 py-1 bg-white/10 backdrop-blur-sm text-white text-xs rounded opacity-0 group-hover:opacity-100 pointer-events-none whitespace-nowrap transition-opacity z-50">
                        {tab.name}
                      </div>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>

          {/* Recent Conversations - shown when Chat tab is active */}
          {activeTab === 'chat' && !sidebarCollapsed && (
            <div className="mt-4 px-3">
              <ConversationsPanel
                selectedConversation={selectedConversation}
                onSelectConversation={setSelectedConversation}
              />
            </div>
          )}
        </nav>

        {/* User Profile Section - Bottom of sidebar */}
        <div className={clsx(
          'border-t border-white/10',
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
                'flex items-center gap-2 rounded-lg transition-all duration-200 hover:bg-white/5',
                sidebarCollapsed ? 'p-1 flex-col' : 'flex-1 min-w-0 p-1.5'
              )}
              title="Open Settings"
              aria-label="Open Settings"
            >
              {/* User Avatar with online indicator */}
              <div className="relative flex-shrink-0">
                <div className="w-8 h-8 bg-gradient-to-br from-pierre-violet to-pierre-cyan rounded-full flex items-center justify-center">
                  <span className="text-xs font-bold text-white">
                    {(user?.display_name || user?.email)?.charAt(0).toUpperCase()}
                  </span>
                </div>
                {/* Online status dot */}
                <div className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 bg-pierre-activity rounded-full border-2 border-pierre-slate" />
              </div>

              {!sidebarCollapsed && (
                <div className="flex-1 min-w-0 text-left">
                  <p className="text-[11px] font-medium text-white truncate leading-tight">
                    {user?.display_name || user?.email}
                  </p>
                  <span className="text-[9px] text-zinc-400 uppercase">
                    {user?.role === 'super_admin' ? 'Super Admin' : user?.role === 'admin' ? 'Admin' : 'User'}
                  </span>
                </div>
              )}
            </button>

            {/* Notification bell */}
            <NotificationBell onViewAll={() => setActiveTab('notifications')} onNavigate={setActiveTab} />

            {/* Settings gear icon - visible shortcut to user settings */}
            <button
              onClick={() => setActiveTab('settings')}
              className={clsx(
                'text-zinc-500 hover:text-pierre-violet transition-colors flex-shrink-0 flex items-center justify-center',
                sidebarCollapsed ? 'min-w-[44px] min-h-[44px]' : 'min-w-[44px] min-h-[44px]',
                activeTab === 'settings' && 'text-pierre-violet'
              )}
              title="Settings"
              aria-label="Settings"
            >
              <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
              </svg>
            </button>

            {/* Sign out button */}
            <button
              onClick={logout}
              className="text-zinc-500 hover:text-pierre-violet transition-colors flex-shrink-0 min-w-[44px] min-h-[44px] flex items-center justify-center"
              title="Sign out"
              aria-label="Sign out"
            >
              <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1" />
              </svg>
            </button>
          </div>
        </div>

        {/* Collapse Toggle Button */}
        <button
          onClick={() => {
            const next = !sidebarCollapsed;
            localStorage.setItem('pierre.sidebar_collapsed', String(next));
            setSidebarCollapsed(next);
          }}
          className="absolute -right-5 top-20 w-11 h-11 bg-pierre-slate border border-white/20 rounded-full flex items-center justify-center shadow-sm hover:bg-white/10 hover:border-pierre-violet transition-all duration-200 z-50"
          title={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          aria-label={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        >
          <svg
            className={clsx(
              'w-4 h-4 text-zinc-400 transition-transform duration-300',
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
      </aside>

      {/* Main Content Area */}
      <main
        className={clsx(
          'flex-1 h-screen flex flex-col transition-all duration-300 ease-in-out',
          sidebarCollapsed ? 'ml-[72px]' : 'ml-[260px]'
        )}
      >
        {/* Top Header Bar - only for admin tabs; user tabs have their own TabHeader */}
        {isAdminUser && (
          <header className="bg-pierre-slate/80 backdrop-blur-lg shadow-sm border-b border-white/10 sticky top-0 z-30 flex-shrink-0">
            <div className="px-6 py-4 flex items-center justify-between">
              <div>
                <h1 className="text-xl font-medium text-white">
                  {tabs.find(t => t.id === activeTab)?.name || (activeTab === 'settings' ? 'Settings' : '')}
                </h1>
              </div>
            </div>
          </header>
        )}

        {/* Content Area - full height, no extra padding for user tabs that manage their own layout */}
        <div className={clsx(
          'flex-1 overflow-auto',
          isAdminUser && activeTab !== 'chat' ? 'p-6' : ''
        )}>

          {/* Content */}
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
          </div>
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
        {activeTab === 'insights' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            {insightsView === 'friends' ? (
              <FriendsTab onBack={() => setInsightsView('feed')} />
            ) : (
              <SocialFeedTab onNavigateToFriends={() => setInsightsView('friends')} />
            )}
          </Suspense>
        )}
        {activeTab === 'chat' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <ChatTab
              selectedConversation={selectedConversation}
              onSelectConversation={setSelectedConversation}
              onNavigateToInsights={() => setActiveTab('insights')}
            />
          </Suspense>
        )}
        {activeTab === 'my-coaches' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <CoachLibraryTab />
          </Suspense>
        )}
        {activeTab === 'discover' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <StoreScreen />
          </Suspense>
        )}
        {activeTab === 'groups' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            {selectedGroupId ? (
              <GroupDetail groupId={selectedGroupId} onBack={() => setSelectedGroupId(null)} />
            ) : (
              <GroupManagement onSelectGroup={setSelectedGroupId} pendingInviteCode={pendingInviteCode ?? undefined} onInviteCodeConsumed={onInviteCodeConsumed} />
            )}
          </Suspense>
        )}
        {activeTab === 'settings' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <UserSettings />
          </Suspense>
        )}
        {activeTab === 'notifications' && (
          <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
            <NotificationsPanel onNavigate={setActiveTab} />
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
                  <h2 className="text-xl font-semibold mb-4 text-white">API Key Management</h2>
                  <p className="text-zinc-400 mb-4">
                    Manage API keys for MCP clients and programmatic access. Only super admins can create, rotate, and revoke API keys.
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
    </div>
  );
}
