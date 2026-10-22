// ABOUTME: Admin settings tab for system configuration
// ABOUTME: Provides the auto-approval toggle, the sign-up guards, group creation policy, and feature flags
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, type FormEvent } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import type { AutoApprovalSetting, SignupSetting, SignupSettingUpdate } from '../services/api/admin';
import { useAuth } from '../hooks/useAuth';
import { useGroupPermissions } from '../hooks/useGroups';
import { Button, Card, ConfirmDialog, Input, Select } from './ui';
import { formatDateTime } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '../constants/queryKeys';
import FeatureFlagsPanel from './FeatureFlagsPanel';
import { featureFlagsQuery } from './featureFlagsQuery';

interface SettingSwitchProps {
  checked: boolean;
  label: string;
  onToggle: () => void;
  disabled?: boolean;
  title?: string;
}

/** The on/off switch every toggle on this page draws. */
function SettingSwitch({ checked, label, onToggle, disabled = false, title }: SettingSwitchProps) {
  return (
    <button
      type="button"
      onClick={onToggle}
      disabled={disabled}
      title={title}
      className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2 focus:ring-offset-surface-container ${
        checked ? 'bg-activity' : 'bg-surface-container-high'
      } ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}
      role="switch"
      aria-checked={checked}
      aria-label={label}
    >
      <span
        className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
          checked ? 'translate-x-6' : 'translate-x-1'
        }`}
      />
    </button>
  );
}

export default function AdminSettings() {
  const queryClient = useQueryClient();
  const { language } = useTranslation();
  const { user } = useAuth();
  const adminTenantId = user?.tenant_id ?? null;

  const { data: autoApprovalData, isLoading, error } = useQuery<AutoApprovalSetting>({
    queryKey: QUERY_KEYS.adminSettings.autoApproval(),
    queryFn: () => adminApi.getAutoApprovalSetting(),
    retry: 1,
  });

  const autoApprovalLockedByEnv = autoApprovalData?.overridden_by_env === true;

  const updateAutoApprovalMutation = useMutation({
    mutationFn: (enabled: boolean) => adminApi.updateAutoApprovalSetting(enabled),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminSettings.autoApproval() });
    },
  });

  // Sign-up guards (carnet#1063): the kill switch, the daily ceiling and
  // activate-on-verified-email.
  const signupQuery = useQuery<SignupSetting>({
    queryKey: QUERY_KEYS.adminSettings.signup(),
    queryFn: () => adminApi.getSignupSetting(),
    retry: 1,
  });
  const signupData = signupQuery.data;

  const updateSignupMutation = useMutation({
    mutationFn: (update: SignupSettingUpdate) => adminApi.updateSignupSetting(update),
    onSuccess: (updated, update) => {
      queryClient.setQueryData(QUERY_KEYS.adminSettings.signup(), updated);
      // Only a saved limit clears the field; flipping a toggle keeps a half-typed one.
      if (update.daily_limit !== undefined) setDailyLimitDraft(null);
    },
  });

  // Restart the daily count from zero, limit unchanged (carnet#1131). Asked
  // for in a dialog first: it lets another day's worth of accounts in.
  const [confirmResetCount, setConfirmResetCount] = useState(false);
  const resetCountMutation = useMutation({
    mutationFn: () => adminApi.resetSignupCount(),
    onSuccess: (updated) => {
      queryClient.setQueryData(QUERY_KEYS.adminSettings.signup(), updated);
      setConfirmResetCount(false);
    },
    onError: () => setConfirmResetCount(false),
  });

  // The ceiling being typed, or null while the field shows the saved value.
  const [dailyLimitDraft, setDailyLimitDraft] = useState<string | null>(null);
  const dailyLimitValue = dailyLimitDraft ?? String(signupData?.daily_limit ?? 0);
  const parsedDailyLimit = /^\d+$/.test(dailyLimitValue.trim()) ? Number(dailyLimitValue.trim()) : null;
  const dailyLimitInvalid = parsedDailyLimit === null;
  const dailyLimitDirty = signupData !== undefined && parsedDailyLimit !== signupData.daily_limit;

  const handleSaveDailyLimit = (e: FormEvent) => {
    e.preventDefault();
    if (parsedDailyLimit !== null && dailyLimitDirty) {
      updateSignupMutation.mutate({ daily_limit: parsedDailyLimit });
    }
  };

  // Group creation policy
  const { policy: groupCreationPolicy, isLoading: groupPolicyLoading } = useGroupPermissions();

  const updateGroupPolicyMutation = useMutation({
    mutationFn: (newPolicy: string) => adminApi.updateConfig({
      parameters: { group_creation_policy: newPolicy },
      reason: 'Admin updated group creation policy',
    }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.permissions() });
    },
  });

  // Hold the page for its first load until the flags, the auto-approval
  // setting, the sign-up guards and the group policy are all in: drawn as each landed, the flags
  // panel at the top could arrive last and push both cards below it off
  // screen (layout shift 0.50 on a phone).
  const flagsQuery = useQuery({
    ...featureFlagsQuery({ kind: 'tenant', id: adminTenantId ?? '' }),
    enabled: Boolean(adminTenantId),
  });
  const [revealed, setRevealed] = useState(false);
  if (!revealed && !isLoading && !signupQuery.isLoading && !groupPolicyLoading && !flagsQuery.isLoading) {
    setRevealed(true);
  }

  const handleToggleAutoApproval = () => {
    if (autoApprovalData && !autoApprovalLockedByEnv) {
      updateAutoApprovalMutation.mutate(!autoApprovalData.enabled);
    }
  };

  if (!revealed) {
    return (
      <div className="flex justify-center py-8" role="status" aria-label="Loading platform settings">
        <div className="pierre-spinner" />
      </div>
    );
  }

  return (
    <div className="space-y-6 max-w-3xl">
      {/* Tenant-wide feature flags. Falls back to an info banner when the
          admin has no tenant_id yet (e.g. bootstrap admin before tenant
          provisioning). */}
      {adminTenantId ? (
        <FeatureFlagsPanel scope={{ kind: 'tenant', id: adminTenantId }} />
      ) : (
        <Card variant="dark">
          <h2 className="text-lg font-semibold text-on-surface mb-2">Feature Flags</h2>
          <p className="text-sm text-on-surface-variant">
            No tenant attached to your admin account — feature flags require a tenant.
          </p>
        </Card>
      )}

      {/* User Registration Settings */}
      <Card variant="dark">
        <h2 className="text-lg font-semibold text-on-surface mb-4">User Registration</h2>

        <div className="space-y-4">
          {/* Auto-Approval Toggle */}
          <div className="flex items-start justify-between p-4 bg-surface-container-low rounded-lg border ghost-border">
            <div className="flex-1 mr-4">
              <h3 className="font-medium text-on-surface">Auto-Approve Registrations</h3>
              <p className="text-sm text-on-surface-variant mt-1">
                {autoApprovalData?.description ??
                  'When enabled, new registrations are auto-approved. When disabled, only emails from auto_approve_domains are auto-approved.'}
              </p>
              {autoApprovalLockedByEnv && (
                <p className="text-xs text-warning mt-2" data-testid="auto-approval-env-lock">
                  Locked by the AUTO_APPROVE_USERS environment variable on the server. Change it
                  there and restart — a value saved here would be ignored.
                </p>
              )}
            </div>
            <div className="flex-shrink-0">
              {isLoading ? (
                <div className="w-11 h-6 bg-surface-container-high rounded-full animate-pulse" />
              ) : error ? (
                <span className="text-xs text-error">Error loading</span>
              ) : (
                <SettingSwitch
                  checked={autoApprovalData?.enabled === true}
                  label="Auto-Approve Registrations"
                  onToggle={handleToggleAutoApproval}
                  disabled={updateAutoApprovalMutation.isPending || autoApprovalLockedByEnv}
                  title={
                    autoApprovalLockedByEnv
                      ? 'Set by the AUTO_APPROVE_USERS environment variable — not editable here'
                      : undefined
                  }
                />
              )}
            </div>
          </div>

          {/* Status indicator */}
          {autoApprovalData && (
            <div className={`flex items-center gap-2 p-3 rounded-lg text-sm ${
              autoApprovalData.enabled
                ? 'bg-activity/15 text-on-activity-container border border-activity/30'
                : 'bg-surface-container-low text-on-surface-variant border ghost-border'
            }`}>
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                {autoApprovalData.enabled ? (
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                ) : (
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                )}
              </svg>
              <span>
                {autoApprovalData.enabled
                  ? 'Auto-approval is enabled. New registrations will be approved automatically.'
                  : 'Auto-approval is disabled. New registrations require admin approval.'}
              </span>
            </div>
          )}

          {/* Mutation status */}
          {updateAutoApprovalMutation.isSuccess && (
            <div className="p-3 rounded-lg bg-activity/15 text-on-activity-container text-sm border border-activity/30">
              Setting updated successfully.
            </div>
          )}
          {updateAutoApprovalMutation.isError && (
            <div className="p-3 rounded-lg bg-error/15 text-error text-sm border border-error/30">
              Failed to update setting. Please try again.
            </div>
          )}

          {/* Sign-up guards (carnet#1063) */}
          <div className="pt-4 border-t ghost-border space-y-4" data-testid="signup-settings">
            <div>
              <h3 className="font-medium text-on-surface">Sign-up</h3>
              <p className="text-sm text-on-surface-variant mt-1">
                {signupData?.description ??
                  'Controls whether new accounts can be created, how many per day, and whether confirming the email activates them.'}
              </p>
            </div>

            {signupQuery.isError ? (
              <p className="text-sm text-error">Error loading the sign-up settings.</p>
            ) : signupData ? (
              <>
                <div className="flex items-start justify-between p-4 bg-surface-container-low rounded-lg border ghost-border">
                  <div className="flex-1 mr-4">
                    <h4 className="font-medium text-on-surface">Sign-up open</h4>
                    <p className="text-sm text-on-surface-variant mt-1">
                      {signupData.open
                        ? 'New accounts can be created.'
                        : 'Sign-up is paused: the register form tells visitors to try again later.'}
                    </p>
                  </div>
                  <div className="flex-shrink-0">
                    <SettingSwitch
                      checked={signupData.open}
                      label="Sign-up open"
                      onToggle={() => updateSignupMutation.mutate({ open: !signupData.open })}
                      disabled={updateSignupMutation.isPending}
                    />
                  </div>
                </div>

                <form
                  onSubmit={handleSaveDailyLimit}
                  className="p-4 bg-surface-container-low rounded-lg border ghost-border"
                >
                  <div className="flex items-end gap-3">
                    <div className="w-40">
                      <Input
                        id="signup-daily-limit"
                        type="number"
                        min={0}
                        step={1}
                        inputMode="numeric"
                        label="Daily sign-up limit"
                        value={dailyLimitValue}
                        onChange={(e) => setDailyLimitDraft(e.target.value)}
                        error={dailyLimitInvalid ? 'Enter a whole number, 0 or more.' : undefined}
                        disabled={updateSignupMutation.isPending}
                        variant="dark"
                      />
                    </div>
                    <Button
                      type="submit"
                      variant="secondary"
                      disabled={dailyLimitInvalid || !dailyLimitDirty || updateSignupMutation.isPending}
                    >
                      Save limit
                    </Button>
                  </div>
                  <p className="text-xs text-on-surface-variant mt-2">
                    Accounts allowed per rolling 24 hours, or since the last reset when that is later. 0 means no ceiling.
                  </p>
                </form>

                <div className="flex items-start justify-between p-4 bg-surface-container-low rounded-lg border ghost-border">
                  <div className="flex-1 mr-4">
                    <h4 className="font-medium text-on-surface">Activate on verified email</h4>
                    <p className="text-sm text-on-surface-variant mt-1">
                      When enabled, confirming the email address activates the account without an admin approval.
                    </p>
                  </div>
                  <div className="flex-shrink-0">
                    <SettingSwitch
                      checked={signupData.activate_on_verified_email}
                      label="Activate on verified email"
                      onToggle={() =>
                        updateSignupMutation.mutate({
                          activate_on_verified_email: !signupData.activate_on_verified_email,
                        })
                      }
                      disabled={updateSignupMutation.isPending}
                    />
                  </div>
                </div>

                <dl className="grid grid-cols-2 gap-3 text-sm">
                  <div className="p-3 bg-surface-container-low rounded-lg border ghost-border">
                    <dt className="text-on-surface-variant">Accounts counted against the limit</dt>
                    <dd className="mt-1 font-medium text-on-surface" data-testid="signup-counted">
                      {signupData.counted}
                      {signupData.daily_limit > 0 ? ` / ${signupData.daily_limit}` : ''}
                    </dd>
                    <dd className="mt-1 text-xs text-on-surface-variant" data-testid="signup-counted-since">
                      counted since {formatDateTime(signupData.counted_since, language)}
                    </dd>
                    <dd className="mt-2">
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        onClick={() => setConfirmResetCount(true)}
                        disabled={resetCountMutation.isPending}
                      >
                        Reset today&apos;s count
                      </Button>
                    </dd>
                  </div>
                  <div className="p-3 bg-surface-container-low rounded-lg border ghost-border">
                    <dt className="text-on-surface-variant">Bot check configured</dt>
                    <dd className="mt-1 font-medium text-on-surface" data-testid="signup-turnstile-configured">
                      {signupData.turnstile_configured ? 'Yes' : 'No'}
                    </dd>
                  </div>
                </dl>

                {updateSignupMutation.isSuccess && (
                  <div className="p-3 rounded-lg bg-activity/15 text-on-activity-container text-sm border border-activity/30">
                    Sign-up settings updated.
                  </div>
                )}
                {updateSignupMutation.isError && (
                  <div className="p-3 rounded-lg bg-error/15 text-error text-sm border border-error/30">
                    Failed to update the sign-up settings. Please try again.
                  </div>
                )}
                {resetCountMutation.isSuccess && (
                  <div
                    role="status"
                    className="p-3 rounded-lg bg-activity/15 text-on-activity-container text-sm border border-activity/30"
                  >
                    Today&apos;s count restarted from zero.
                  </div>
                )}
                {resetCountMutation.isError && (
                  <div
                    role="alert"
                    className="p-3 rounded-lg bg-error/15 text-error text-sm border border-error/30"
                  >
                    Failed to reset the count. Please try again.
                  </div>
                )}
                <ConfirmDialog
                  isOpen={confirmResetCount}
                  onClose={() => setConfirmResetCount(false)}
                  onConfirm={() => resetCountMutation.mutate()}
                  title="Reset today's count?"
                  message={
                    signupData.daily_limit > 0
                      ? `The count restarts from zero now, so another ${signupData.daily_limit} accounts can be created before the ceiling stops sign-ups again. The limit itself is unchanged.`
                      : 'The count restarts from zero now. No daily limit is set, so this only resets the figure.'
                  }
                  confirmLabel="Reset count"
                  variant="warning"
                  isLoading={resetCountMutation.isPending}
                />
              </>
            ) : null}
          </div>
        </div>
      </Card>


      {/* Group Creation Policy */}
      <Card variant="dark">
        <h2 className="text-lg font-semibold text-on-surface mb-4">Group Permissions</h2>

        <div className="space-y-4">
          <div className="p-4 bg-surface-container-low rounded-lg border ghost-border">
            <div className="flex-1 mr-4 mb-3">
              <h3 className="font-medium text-on-surface">Group Creation Policy</h3>
              <p className="text-sm text-on-surface-variant mt-1">
                Controls who can create coaching groups within the tenant.
              </p>
            </div>
            {groupPolicyLoading ? (
              <div className="w-48 h-10 bg-surface-container-high rounded-lg animate-pulse" />
            ) : (
              <div className="w-48">
                <Select
                  aria-label="Group Creation Policy"
                  value={groupCreationPolicy}
                  onChange={(e) => updateGroupPolicyMutation.mutate(e.target.value)}
                  disabled={updateGroupPolicyMutation.isPending}
                  options={[
                    { value: 'admins_only', label: 'Admins Only' },
                    { value: 'everyone', label: 'Everyone' },
                  ]}
                />
              </div>
            )}
          </div>

          {/* Status indicator */}
          <div className={`flex items-center gap-2 p-3 rounded-lg text-sm ${
            groupCreationPolicy === 'everyone'
              ? 'bg-activity/15 text-on-activity-container border border-activity/30'
              : 'bg-surface-container-low text-on-surface-variant border ghost-border'
          }`}>
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              {groupCreationPolicy === 'everyone' ? (
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
              ) : (
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
              )}
            </svg>
            <span>
              {groupCreationPolicy === 'everyone'
                ? 'All users can create coaching groups.'
                : 'Only tenant admins and owners can create coaching groups.'}
            </span>
          </div>

          {/* Mutation status */}
          {updateGroupPolicyMutation.isSuccess && (
            <div className="p-3 rounded-lg bg-activity/15 text-on-activity-container text-sm border border-activity/30">
              Group creation policy updated successfully.
            </div>
          )}
          {updateGroupPolicyMutation.isError && (
            <div className="p-3 rounded-lg bg-error/15 text-error text-sm border border-error/30">
              Failed to update policy. Please try again.
            </div>
          )}
        </div>
      </Card>
    </div>
  );
}
