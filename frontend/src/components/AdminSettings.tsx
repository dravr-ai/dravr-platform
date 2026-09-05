// ABOUTME: Admin settings tab for system configuration
// ABOUTME: Provides the auto-approval toggle, group creation policy, and feature flags
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import { useAuth } from '../hooks/useAuth';
import { useGroupPermissions } from '../hooks/useGroups';
import { Card, Select } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import FeatureFlagsPanel from './FeatureFlagsPanel';

/** Auto-approval payload from `GET /api/admin/settings/auto-approval`. */
interface AutoApprovalSetting {
  enabled: boolean;
  description: string;
  /**
   * True when the server's AUTO_APPROVE_USERS environment variable decides
   * `enabled`. The database row the toggle writes is inert while it is set,
   * so the toggle is rendered read-only. Absent on servers that do not report
   * the override, which are treated as not overridden.
   */
  overridden_by_env?: boolean;
}

export default function AdminSettings() {
  const queryClient = useQueryClient();
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

  const handleToggleAutoApproval = () => {
    if (autoApprovalData && !autoApprovalLockedByEnv) {
      updateAutoApprovalMutation.mutate(!autoApprovalData.enabled);
    }
  };

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
                <button
                  onClick={handleToggleAutoApproval}
                  disabled={updateAutoApprovalMutation.isPending || autoApprovalLockedByEnv}
                  title={
                    autoApprovalLockedByEnv
                      ? 'Set by the AUTO_APPROVE_USERS environment variable — not editable here'
                      : undefined
                  }
                  className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2 focus:ring-offset-surface-container ${
                    autoApprovalData?.enabled
                      ? 'bg-activity'
                      : 'bg-surface-container-high'
                  } ${updateAutoApprovalMutation.isPending || autoApprovalLockedByEnv ? 'opacity-50 cursor-not-allowed' : ''}`}
                  role="switch"
                  aria-checked={autoApprovalData?.enabled}
                >
                  <span
                    className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                      autoApprovalData?.enabled ? 'translate-x-6' : 'translate-x-1'
                    }`}
                  />
                </button>
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
