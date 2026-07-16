// ABOUTME: Sliding drawer component that shows detailed user information
// ABOUTME: Displays user profile, rate limits, activity, and admin actions (approve, suspend, impersonate)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import type { User, UserTier } from '../types/api';
import { Button, Card } from './ui';
import { Badge } from './ui/Badge';
import PasswordResetModal from './PasswordResetModal';
import { useAuth } from '../hooks/useAuth';
import { QUERY_KEYS } from '../constants/queryKeys';
import FeatureFlagsPanel from './FeatureFlagsPanel';

interface UserDetailDrawerProps {
  user: User | null;
  isOpen: boolean;
  onClose: () => void;
  onAction: (user: User, action: 'approve' | 'suspend') => void;
}

export default function UserDetailDrawer({
  user,
  isOpen,
  onClose,
  onAction
}: UserDetailDrawerProps) {
  const [isResetModalOpen, setIsResetModalOpen] = useState(false);
  const [isImpersonating, setIsImpersonating] = useState(false);
  const [overrideEditing, setOverrideEditing] = useState(false);
  const [overrideDaily, setOverrideDaily] = useState<string>('');
  const [overrideMonthly, setOverrideMonthly] = useState<string>('');
  const [overrideNote, setOverrideNote] = useState<string>('');
  const [tierEditing, setTierEditing] = useState(false);
  const [tierValue, setTierValue] = useState<UserTier>('starter');
  const { user: currentUser, startImpersonation } = useAuth();
  const queryClient = useQueryClient();

  const isSuperAdmin = currentUser?.role === 'super_admin';

  const canImpersonate = isSuperAdmin &&
    user?.role !== 'super_admin' &&
    user?.user_status === 'active';

  const handleImpersonate = async () => {
    if (!user || !canImpersonate) return;

    setIsImpersonating(true);
    try {
      await startImpersonation(user.id, `Viewing user account from admin panel`);
      onClose();
    } catch (error) {
      console.error('Failed to start impersonation:', error);
    } finally {
      setIsImpersonating(false);
    }
  };

  const { data: rateLimit, isLoading: rateLimitLoading } = useQuery({
    queryKey: QUERY_KEYS.adminUsers.rateLimit(user?.id),
    queryFn: () => user ? adminApi.getUserRateLimit(user.id) : null,
    enabled: !!user && isOpen,
  });

  const { data: activity, isLoading: activityLoading } = useQuery({
    queryKey: QUERY_KEYS.adminUsers.activity(user?.id),
    queryFn: () => user ? adminApi.getUserActivity(user.id, 30) : null,
    enabled: !!user && isOpen,
  });

  const { data: usage, isLoading: usageLoading } = useQuery({
    queryKey: ['adminUsers', 'usage', user?.id],
    queryFn: () => user ? adminApi.getUserUsage(user.id) : null,
    enabled: !!user && isOpen,
  });

  const { data: adminProfile, isLoading: adminProfileLoading } = useQuery({
    queryKey: ['adminUsers', 'admin-profile', user?.id],
    queryFn: () => user ? adminApi.getUserAdminProfile(user.id) : null,
    enabled: !!user && isOpen,
  });

  const formatCost = (usd: number) => {
    if (usd < 0.01 && usd > 0) return '<$0.01';
    return `$${usd.toFixed(2)}`;
  };

  const formatPersona = (slug: string) =>
    slug.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());

  const refetchRateLimit = () => {
    queryClient.invalidateQueries({
      queryKey: QUERY_KEYS.adminUsers.rateLimit(user?.id),
    });
  };

  const setOverrideMutation = useMutation({
    mutationFn: () => {
      if (!user) return Promise.reject(new Error('no user'));
      const parseLimit = (v: string): number | null => {
        const trimmed = v.trim();
        if (trimmed === '' || trimmed.toLowerCase() === 'unlimited') return null;
        const n = Number.parseInt(trimmed, 10);
        if (!Number.isFinite(n) || n <= 0) {
          throw new Error('Daily and monthly limits must be positive integers or blank for unlimited');
        }
        return n;
      };
      return adminApi.setUserRateLimitOverride(user.id, {
        daily_limit: parseLimit(overrideDaily),
        monthly_limit: parseLimit(overrideMonthly),
        note: overrideNote.trim() === '' ? null : overrideNote.trim(),
      });
    },
    onSuccess: () => {
      setOverrideEditing(false);
      refetchRateLimit();
    },
  });

  const clearOverrideMutation = useMutation({
    mutationFn: () =>
      user
        ? adminApi.clearUserRateLimitOverride(user.id)
        : Promise.reject(new Error('no user')),
    onSuccess: () => {
      setOverrideEditing(false);
      refetchRateLimit();
    },
  });

  // Tier changes cascade to the users list and rate limits (limits derive from tier)
  const refetchAfterTierChange = () => {
    setTierEditing(false);
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminUsers.list() });
    refetchRateLimit();
  };

  const setTierMutation = useMutation({
    mutationFn: (tier: UserTier) =>
      user ? adminApi.setUserTier(user.id, tier) : Promise.reject(new Error('no user')),
    onSuccess: refetchAfterTierChange,
  });

  const clearTierMutation = useMutation({
    mutationFn: () =>
      user ? adminApi.clearUserTier(user.id) : Promise.reject(new Error('no user')),
    onSuccess: refetchAfterTierChange,
  });

  if (!isOpen || !user) return null;

  const formatDate = (dateString: string) => {
    return new Date(dateString).toLocaleDateString('en-US', {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit'
    });
  };

  const formatLimit = (limit: number | null) => {
    if (limit === null) return 'Unlimited';
    return limit.toLocaleString();
  };

  const getUsagePercentage = (used: number, limit: number | null) => {
    if (limit === null || limit === 0) return 0;
    return Math.min(100, (used / limit) * 100);
  };

  const getUsageColor = (percentage: number) => {
    if (percentage >= 90) return 'bg-pierre-red-400';
    if (percentage >= 75) return 'bg-pierre-nutrition';
    return 'bg-pierre-activity';
  };

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 bg-black bg-opacity-50 z-40"
        onClick={onClose}
      />

      {/* Drawer */}
      <div className="fixed inset-y-0 right-0 w-full max-w-md bg-surface-container-low shadow-xl z-50 overflow-y-auto">
        {/* Header */}
        <div className="sticky top-0 bg-surface-container-low border-b ghost-border px-6 py-4 flex justify-between items-center">
          <h2 className="text-xl font-semibold text-on-surface">User Details</h2>
          <button
            onClick={onClose}
            aria-label="Close drawer"
            className="text-on-surface-variant hover:text-on-surface"
          >
            <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="p-6 space-y-6">
          {/* User Info Card */}
          <Card variant="dark" className="p-4">
            <div className="flex items-start justify-between mb-4">
              <div>
                <h3 className="text-lg font-semibold text-on-surface">
                  {user.display_name || 'Unnamed User'}
                </h3>
                <p className="text-sm text-on-surface-variant">{user.email}</p>
              </div>
              <div className="flex flex-col items-end space-y-2">
                <Badge
                  variant={
                    user.user_status === 'pending' ? 'warning' :
                    user.user_status === 'active' ? 'success' : 'destructive'
                  }
                >
                  {user.user_status}
                </Badge>
                <span className="text-xs text-on-surface capitalize bg-surface-container-high px-2 py-1 rounded border ghost-border">
                  {user.tier}
                </span>
                {isSuperAdmin && !tierEditing && (
                  <button
                    type="button"
                    onClick={() => {
                      // The users list serves the display form ("Professional");
                      // the select values (and the API) are lowercase. Without
                      // normalizing, the select silently falls back to Starter
                      // and a blind Save would downgrade the user.
                      const normalized = user.tier.toLowerCase();
                      setTierValue(
                        normalized === 'professional' || normalized === 'enterprise'
                          ? normalized
                          : 'starter'
                      );
                      setTierEditing(true);
                    }}
                    className="text-xs text-pierre-activity hover:underline"
                  >
                    Edit tier
                  </button>
                )}
              </div>
            </div>

            {isSuperAdmin && tierEditing && (
              <div className="mb-4 rounded-md bg-surface-container-high p-3 space-y-2 text-xs">
                <div className="font-semibold text-on-surface">Tier override</div>
                <p className="text-on-surface-variant">
                  Sets the billing/quota tier directly. Clearing the override lets the
                  billing webhook drive the tier again.
                </p>
                <label className="block">
                  <span className="text-on-surface-variant">Tier</span>
                  <select
                    value={tierValue}
                    onChange={(e) => setTierValue(e.target.value as UserTier)}
                    aria-label="Tier"
                    className="mt-1 w-full rounded bg-surface-container-low border ghost-border px-2 py-1 text-on-surface"
                  >
                    <option value="starter">Starter</option>
                    <option value="professional">Professional</option>
                    <option value="enterprise">Enterprise</option>
                  </select>
                </label>
                {(setTierMutation.isError || clearTierMutation.isError) && (
                  <p className="text-pierre-red-400">
                    Failed to update tier. Please try again.
                  </p>
                )}
                <div className="flex items-center gap-2 pt-1">
                  <Button
                    onClick={() => setTierMutation.mutate(tierValue)}
                    disabled={setTierMutation.isPending}
                    className="bg-pierre-activity text-on-primary"
                  >
                    {setTierMutation.isPending ? 'Saving…' : 'Save tier'}
                  </Button>
                  <Button
                    onClick={() => clearTierMutation.mutate()}
                    disabled={clearTierMutation.isPending}
                    variant="secondary"
                  >
                    {clearTierMutation.isPending ? 'Clearing…' : 'Clear override'}
                  </Button>
                  <Button onClick={() => setTierEditing(false)} variant="secondary">
                    Cancel
                  </Button>
                </div>
              </div>
            )}

            <div className="grid grid-cols-2 gap-4 text-sm">
              <div>
                <span className="text-on-surface-variant">Registered</span>
                <p className="font-medium text-on-surface">{formatDate(user.created_at)}</p>
              </div>
              <div>
                <span className="text-on-surface-variant">Last Active</span>
                <p className="font-medium text-on-surface">{user.last_active ? formatDate(user.last_active) : 'Never'}</p>
              </div>
              {user.approved_at && (
                <div>
                  <span className="text-on-surface-variant">Approved</span>
                  <p className="font-medium text-on-surface">{formatDate(user.approved_at)}</p>
                </div>
              )}
              {user.approved_by && (
                <div>
                  <span className="text-on-surface-variant">Approved By</span>
                  <p className="font-medium text-on-surface">{user.approved_by}</p>
                </div>
              )}
              {adminProfile && (
                <div>
                  <span className="text-on-surface-variant">Coach Style</span>
                  <p className="font-medium text-on-surface">
                    {formatPersona(adminProfile.coaching_persona)}
                  </p>
                </div>
              )}
              {adminProfile?.default_coach_id && (
                <div>
                  <span className="text-on-surface-variant">Default Coach</span>
                  <p className="font-medium text-on-surface truncate" title={adminProfile.default_coach_id}>
                    {adminProfile.default_coach_id}
                  </p>
                </div>
              )}
            </div>
          </Card>

          {/* Rate Limits Card */}
          <Card variant="dark" className="p-4">
            <h4 className="text-sm font-semibold text-on-surface mb-4 flex items-center">
              <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
              </svg>
              Rate Limits
            </h4>

            {rateLimitLoading ? (
              <div className="space-y-3 animate-pulse">
                <div className="h-4 bg-surface-container-high rounded w-3/4"></div>
                <div className="h-2 bg-surface-container-high rounded w-full"></div>
                <div className="h-4 bg-surface-container-high rounded w-3/4"></div>
                <div className="h-2 bg-surface-container-high rounded w-full"></div>
              </div>
            ) : rateLimit ? (
              <div className="space-y-4">
                {/* Daily Usage */}
                <div>
                  <div className="flex justify-between text-sm mb-1">
                    <span className="text-on-surface-variant">Daily Usage</span>
                    <span className="font-medium text-on-surface">
                      {rateLimit.rate_limits.daily.used.toLocaleString()} / {formatLimit(rateLimit.rate_limits.daily.limit)}
                    </span>
                  </div>
                  <div className="w-full bg-surface-container-high rounded-full h-2">
                    <div
                      className={`h-2 rounded-full transition-all ${getUsageColor(getUsagePercentage(rateLimit.rate_limits.daily.used, rateLimit.rate_limits.daily.limit))}`}
                      style={{ width: `${getUsagePercentage(rateLimit.rate_limits.daily.used, rateLimit.rate_limits.daily.limit)}%` }}
                    />
                  </div>
                  <p className="text-xs text-outline mt-1">
                    Resets: {formatDate(rateLimit.reset_times.daily_reset)}
                  </p>
                </div>

                {/* Monthly Usage */}
                <div>
                  <div className="flex justify-between text-sm mb-1">
                    <span className="text-on-surface-variant">Monthly Usage</span>
                    <span className="font-medium text-on-surface">
                      {rateLimit.rate_limits.monthly.used.toLocaleString()} / {formatLimit(rateLimit.rate_limits.monthly.limit)}
                    </span>
                  </div>
                  <div className="w-full bg-surface-container-high rounded-full h-2">
                    <div
                      className={`h-2 rounded-full transition-all ${getUsageColor(getUsagePercentage(rateLimit.rate_limits.monthly.used, rateLimit.rate_limits.monthly.limit))}`}
                      style={{ width: `${getUsagePercentage(rateLimit.rate_limits.monthly.used, rateLimit.rate_limits.monthly.limit)}%` }}
                    />
                  </div>
                  <p className="text-xs text-outline mt-1">
                    Resets: {formatDate(rateLimit.reset_times.monthly_reset)}
                  </p>
                </div>
                <div className="pt-2 border-t ghost-border space-y-2">
                  {rateLimit.override_active ? (
                    <div className="rounded-md bg-pierre-activity/10 border border-pierre-activity/30 p-2 text-xs">
                      <div className="flex items-center justify-between">
                        <span className="font-medium text-pierre-activity">Per-user override active</span>
                        <button
                          type="button"
                          onClick={() => {
                            setOverrideDaily(rateLimit.rate_limits.daily.limit?.toString() ?? '');
                            setOverrideMonthly(rateLimit.rate_limits.monthly.limit?.toString() ?? '');
                            setOverrideNote(rateLimit.override_note ?? '');
                            setOverrideEditing(true);
                          }}
                          className="text-pierre-activity hover:underline"
                        >
                          Edit
                        </button>
                      </div>
                      {rateLimit.override_note && (
                        <p className="text-on-surface-variant mt-1 italic">"{rateLimit.override_note}"</p>
                      )}
                    </div>
                  ) : (
                    <button
                      type="button"
                      onClick={() => {
                        setOverrideDaily('');
                        setOverrideMonthly('');
                        setOverrideNote('');
                        setOverrideEditing(true);
                      }}
                      className="text-xs text-pierre-activity hover:underline"
                    >
                      Override limits for this user…
                    </button>
                  )}
                  <p className="text-xs text-outline">
                    Tier defaults editable in{' '}
                    <a href="#platform-settings" className="text-pierre-activity hover:underline">
                      Platform Settings → Rate Limits
                    </a>
                    .
                  </p>

                  {overrideEditing && (
                    <div className="rounded-md bg-surface-container-high p-3 space-y-2 text-xs">
                      <div className="font-semibold text-on-surface">Override</div>
                      <p className="text-on-surface-variant">
                        Leave blank for unlimited; positive integers set a custom cap.
                        Industry standard exemption pattern — tier defaults apply when no override is set.
                      </p>
                      <label className="block">
                        <span className="text-on-surface-variant">Daily limit</span>
                        <input
                          type="text"
                          value={overrideDaily}
                          onChange={(e) => setOverrideDaily(e.target.value)}
                          placeholder="e.g. 100, or blank for unlimited"
                          className="mt-1 w-full rounded bg-surface-container-low border ghost-border px-2 py-1 text-on-surface"
                        />
                      </label>
                      <label className="block">
                        <span className="text-on-surface-variant">Monthly limit</span>
                        <input
                          type="text"
                          value={overrideMonthly}
                          onChange={(e) => setOverrideMonthly(e.target.value)}
                          placeholder="e.g. 3000, or blank for unlimited"
                          className="mt-1 w-full rounded bg-surface-container-low border ghost-border px-2 py-1 text-on-surface"
                        />
                      </label>
                      <label className="block">
                        <span className="text-on-surface-variant">Note (operator-only)</span>
                        <input
                          type="text"
                          value={overrideNote}
                          onChange={(e) => setOverrideNote(e.target.value)}
                          placeholder="Why this override exists"
                          className="mt-1 w-full rounded bg-surface-container-low border ghost-border px-2 py-1 text-on-surface"
                        />
                      </label>
                      {setOverrideMutation.error && (
                        <p className="text-pierre-red-400">
                          {(setOverrideMutation.error as Error).message}
                        </p>
                      )}
                      <div className="flex items-center gap-2 pt-1">
                        <Button
                          onClick={() => setOverrideMutation.mutate()}
                          disabled={setOverrideMutation.isPending}
                          className="bg-pierre-activity text-on-primary"
                        >
                          {setOverrideMutation.isPending ? 'Saving…' : 'Save override'}
                        </Button>
                        {rateLimit.override_active && (
                          <Button
                            onClick={() => clearOverrideMutation.mutate()}
                            disabled={clearOverrideMutation.isPending}
                            variant="secondary"
                          >
                            {clearOverrideMutation.isPending ? 'Clearing…' : 'Clear override'}
                          </Button>
                        )}
                        <Button onClick={() => setOverrideEditing(false)} variant="secondary">
                          Cancel
                        </Button>
                      </div>
                    </div>
                  )}
                </div>
              </div>
            ) : (
              <p className="text-sm text-on-surface-variant">Unable to load rate limit data</p>
            )}
          </Card>

          {/* Feature Flags Card — per-user overrides */}
          {user && <FeatureFlagsPanel scope={{ kind: 'user', id: user.id }} />}

          {/* Activity Card */}
          <Card variant="dark" className="p-4">
            <h4 className="text-sm font-semibold text-on-surface mb-4 flex items-center">
              <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
              </svg>
              Activity (Last 30 Days)
            </h4>

            {activityLoading ? (
              <div className="space-y-3 animate-pulse">
                <div className="h-4 bg-surface-container-high rounded w-1/2"></div>
                <div className="h-4 bg-surface-container-high rounded w-3/4"></div>
                <div className="h-4 bg-surface-container-high rounded w-2/3"></div>
              </div>
            ) : activity ? (
              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <span className="text-on-surface-variant">Total Requests</span>
                  <span className="text-2xl font-bold text-on-surface">
                    {activity.total_requests.toLocaleString()}
                  </span>
                </div>

                {activity.top_tools.length > 0 ? (
                  <div>
                    <span className="text-sm text-on-surface-variant">Top Tools</span>
                    <div className="mt-2 space-y-2">
                      {activity.top_tools.slice(0, 5).map((tool) => (
                        <div key={tool.tool_name} className="flex items-center justify-between">
                          <span className="text-sm font-medium text-on-surface">{tool.tool_name}</span>
                          <div className="flex items-center space-x-2">
                            <span className="text-sm text-on-surface-variant">{tool.call_count.toLocaleString()}</span>
                            <span className="text-xs text-outline">({tool.percentage.toFixed(1)}%)</span>
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                ) : (
                  <p className="text-sm text-on-surface-variant">No tool usage in the last 30 days</p>
                )}
              </div>
            ) : (
              <p className="text-sm text-on-surface-variant">Unable to load activity data</p>
            )}
          </Card>

          {/* Usage Card — Phase 2 per-user billing surface */}
          <Card variant="dark" className="p-4">
            <h4 className="text-sm font-semibold text-on-surface mb-4 flex items-center">
              <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 3v18h18M7 14l4-4 4 4 5-5" />
              </svg>
              Usage (Month-to-Date)
            </h4>
            {usageLoading ? (
              <div className="space-y-3 animate-pulse">
                <div className="h-4 bg-surface-container-high rounded w-2/3"></div>
                <div className="h-4 bg-surface-container-high rounded w-1/2"></div>
                <div className="h-4 bg-surface-container-high rounded w-3/4"></div>
              </div>
            ) : usage && Array.isArray(usage.by_model) && usage.by_model.length > 0 ? (
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <span className="text-on-surface-variant">Total Tokens</span>
                  <span className="text-2xl font-bold text-on-surface">
                    {usage.by_model
                      .reduce((acc, m) => acc + m.total_tokens, 0)
                      .toLocaleString()}
                  </span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-on-surface-variant">Total Cost</span>
                  <span className="font-medium text-on-surface">
                    {formatCost(usage.total_cost_usd ?? 0)}
                  </span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-on-surface-variant">Total Calls</span>
                  <span className="font-medium text-on-surface">
                    {usage.by_model.reduce((acc, m) => acc + m.calls, 0).toLocaleString()}
                  </span>
                </div>
                <div>
                  <span className="text-sm text-on-surface-variant">By Model</span>
                  <div className="mt-2 space-y-2">
                    {usage.by_model.slice(0, 5).map((row) => (
                      <div
                        key={`${row.provider}/${row.model}/${row.call_type}`}
                        className="flex items-center justify-between"
                      >
                        <span className="text-sm font-medium text-on-surface">
                          {row.provider}/{row.model}
                        </span>
                        <div className="flex items-center space-x-2">
                          <span className="text-sm text-on-surface-variant">
                            {row.total_tokens.toLocaleString()} tok
                          </span>
                          <span className="text-xs text-outline">
                            ({row.calls} calls · {formatCost(row.cost_usd ?? 0)})
                          </span>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            ) : (
              <p className="text-sm text-on-surface-variant">No LLM activity this month</p>
            )}
          </Card>

          {/* Installed Coaches Card */}
          <Card variant="dark" className="p-4">
            <h4 className="text-sm font-semibold text-on-surface mb-4 flex items-center">
              <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4.354a4 4 0 110 5.292M15 21H3v-1a6 6 0 0112 0v1zm0 0h6v-1a6 6 0 00-9-5.197M13 7a4 4 0 11-8 0 4 4 0 018 0z" />
              </svg>
              Installed Coaches
            </h4>
            {adminProfileLoading ? (
              <div className="space-y-3 animate-pulse">
                <div className="h-4 bg-surface-container-high rounded w-3/4"></div>
                <div className="h-4 bg-surface-container-high rounded w-2/3"></div>
              </div>
            ) : adminProfile && adminProfile.installed_coaches.length > 0 ? (
              <div className="space-y-2">
                {adminProfile.installed_coaches.map((coach) => (
                  <div key={coach.coach_id} className="flex items-center justify-between">
                    <span className="text-sm font-medium text-on-surface">
                      {coach.title}
                      {coach.is_default && (
                        <span className="ml-2 text-xs text-pierre-activity">(default)</span>
                      )}
                    </span>
                    <span className="text-xs text-outline capitalize">{coach.category}</span>
                  </div>
                ))}
              </div>
            ) : (
              <p className="text-sm text-on-surface-variant">No coaches installed</p>
            )}
          </Card>

          {/* Groups Card */}
          <Card variant="dark" className="p-4">
            <h4 className="text-sm font-semibold text-on-surface mb-4 flex items-center">
              <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" />
              </svg>
              Coaching Groups
            </h4>
            {adminProfileLoading ? (
              <div className="space-y-3 animate-pulse">
                <div className="h-4 bg-surface-container-high rounded w-3/4"></div>
                <div className="h-4 bg-surface-container-high rounded w-2/3"></div>
              </div>
            ) : adminProfile && adminProfile.joined_groups.length > 0 ? (
              <div className="space-y-2">
                {adminProfile.joined_groups.map((group) => (
                  <div key={group.group_id} className="flex items-center justify-between">
                    <span className="text-sm font-medium text-on-surface">{group.name}</span>
                    <div className="flex items-center space-x-2">
                      <span className="text-xs text-on-surface-variant capitalize">{group.role}</span>
                      <span className="text-xs text-outline">({group.member_count} members)</span>
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <p className="text-sm text-on-surface-variant">Not a member of any groups</p>
            )}
          </Card>

          {/* Actions */}
          <Card variant="dark" className="p-4">
            <h4 className="text-sm font-semibold text-on-surface mb-4">Actions</h4>
            <div className="space-y-2">
              <Button
                onClick={() => setIsResetModalOpen(true)}
                variant="secondary"
                className="w-full justify-start"
              >
                <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
                </svg>
                Reset Password
              </Button>

              {user.user_status === 'pending' && (
                <Button
                  onClick={() => onAction(user, 'approve')}
                  className="w-full justify-start bg-pierre-activity hover:bg-pierre-activity/80 text-on-primary"
                >
                  <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
                  </svg>
                  Approve User
                </Button>
              )}

              {user.user_status === 'active' && (
                <Button
                  onClick={() => onAction(user, 'suspend')}
                  variant="secondary"
                  className="w-full justify-start border-pierre-red-500/30 text-pierre-red-400 hover:bg-pierre-red-500/10"
                >
                  <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728L5.636 5.636m12.728 12.728L18.364 5.636M5.636 18.364l12.728-12.728" />
                  </svg>
                  Suspend User
                </Button>
              )}

              {user.user_status === 'suspended' && (
                <Button
                  onClick={() => onAction(user, 'approve')}
                  className="w-full justify-start bg-pierre-activity hover:bg-pierre-activity/80 text-on-primary"
                >
                  <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
                  </svg>
                  Reactivate User
                </Button>
              )}

              {canImpersonate && (
                <Button
                  onClick={handleImpersonate}
                  disabled={isImpersonating}
                  variant="secondary"
                  className="w-full justify-start border-pierre-nutrition/30 text-pierre-nutrition hover:bg-pierre-nutrition/10"
                >
                  <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" />
                  </svg>
                  {isImpersonating ? 'Starting...' : 'Impersonate User'}
                </Button>
              )}
            </div>
          </Card>
        </div>
      </div>

      {/* Password Reset Modal */}
      <PasswordResetModal
        user={user}
        isOpen={isResetModalOpen}
        onClose={() => setIsResetModalOpen(false)}
      />
    </>
  );
}
