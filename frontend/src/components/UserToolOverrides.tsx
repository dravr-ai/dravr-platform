// ABOUTME: Per-user MCP tool allow/deny panel for the admin console
// ABOUTME: Search/select a user, then view their effective tools and set/remove per-user overrides
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import type { UserEffectiveTool, UserToolSource } from '../services/api/admin';
import type { User } from '../types/api';
import { Card, Input, Button } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';

// Per-user effective-tools query key (per-tenant keys live in QUERY_KEYS.adminTools)
const userToolsQueryKey = (userId: string) => ['user-tools', userId] as const;

interface SourceBadgeSpec {
  label: string;
  classes: string;
  title: string;
}

// Color-coded provenance badges for the effective enablement decision
const SOURCE_BADGES: Record<UserToolSource, SourceBadgeSpec> = {
  user_override: {
    label: 'User Override',
    classes: 'bg-info/15 text-on-info-container',
    title: 'Per-user admin override — takes precedence over plan and tenant settings',
  },
  plan_restriction: {
    label: 'Plan Restricted',
    classes: 'bg-warning/15 text-on-warning-container',
    title: "Disabled because the tenant's plan does not meet this tool's minimum plan",
  },
  global_disabled: {
    label: 'Global Block',
    classes: 'bg-error/15 text-error',
    title: 'Disabled globally via PIERRE_DISABLED_TOOLS — cannot be enabled for any user',
  },
  tenant_override: {
    label: 'Tenant Override',
    classes: 'bg-info/15 text-on-info-container',
    title: 'Set by a tenant-level override (Tool Management tab)',
  },
  default: {
    label: 'Default',
    classes: 'bg-surface-container-high/15 text-on-surface-variant',
    title: 'Catalog default — no override in effect',
  },
};

function sourceBadge(source: UserToolSource) {
  const spec = SOURCE_BADGES[source] ?? SOURCE_BADGES.default;
  return (
    <span
      title={spec.title}
      className={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ${spec.classes}`}
    >
      {spec.label}
    </span>
  );
}

export default function UserToolOverrides() {
  const queryClient = useQueryClient();
  const [userSearch, setUserSearch] = useState('');
  const [selectedUser, setSelectedUser] = useState<User | null>(null);
  const [toolSearch, setToolSearch] = useState('');

  const selectedUserId = selectedUser?.id ?? '';

  // Admin users listing — same source as UserManagement
  const { data: allUsers = [], isLoading: usersLoading } = useQuery<User[]>({
    queryKey: QUERY_KEYS.adminUsers.list(),
    queryFn: () => adminApi.getAllUsers(),
    retry: 1,
  });

  const matchingUsers = useMemo(() => {
    const query = userSearch.trim().toLowerCase();
    const users = query
      ? allUsers.filter(
          (u) =>
            u.email.toLowerCase().includes(query) ||
            (u.display_name?.toLowerCase().includes(query) ?? false)
        )
      : allUsers;
    return users.slice(0, 25);
  }, [allUsers, userSearch]);

  // Effective tools for the selected user (tenant computation + per-user overlay)
  const {
    data: userToolsData,
    isLoading: toolsLoading,
    error: toolsError,
  } = useQuery({
    queryKey: userToolsQueryKey(selectedUserId),
    queryFn: () => adminApi.getUserTools(selectedUserId),
    retry: 1,
    enabled: !!selectedUserId,
  });

  const setOverrideMutation = useMutation({
    mutationFn: ({ toolName, isEnabled }: { toolName: string; isEnabled: boolean }) =>
      adminApi.setUserToolOverride(selectedUserId, toolName, isEnabled),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: userToolsQueryKey(selectedUserId) });
    },
  });

  const removeOverrideMutation = useMutation({
    mutationFn: (toolName: string) => adminApi.removeUserToolOverride(selectedUserId, toolName),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: userToolsQueryKey(selectedUserId) });
    },
  });

  // Filter by search, then group by category for sectioned rendering.
  // Admin-category tools are excluded: they are role-gated (the registry
  // strips ADMIN_ONLY tools from a non-admin's tools/list regardless of
  // catalog state), so showing them here as "enabled" for a regular user
  // would misstate what the user can actually invoke. Tenant-wide admin
  // tool config lives in the Tool Management tab.
  const toolsByCategory = useMemo(() => {
    const tools = (userToolsData?.data ?? []).filter(
      (tool) => tool.category !== 'admin'
    );
    const query = toolSearch.trim().toLowerCase();
    const filtered = query
      ? tools.filter(
          (tool) =>
            tool.tool_name.toLowerCase().includes(query) ||
            tool.display_name.toLowerCase().includes(query) ||
            tool.description.toLowerCase().includes(query)
        )
      : tools;

    const groups = new Map<string, UserEffectiveTool[]>();
    for (const tool of filtered) {
      const group = groups.get(tool.category);
      if (group) {
        group.push(tool);
      } else {
        groups.set(tool.category, [tool]);
      }
    }
    return Array.from(groups.entries()).sort(([a], [b]) => a.localeCompare(b));
  }, [userToolsData, toolSearch]);

  const handleToggleTool = (tool: UserEffectiveTool) => {
    if (tool.source === 'global_disabled') {
      return; // Backend keeps globally disabled tools off regardless
    }
    setOverrideMutation.mutate({ toolName: tool.tool_name, isEnabled: !tool.is_enabled });
  };

  return (
    <div className="space-y-6">
      {/* User picker */}
      <Card variant="dark">
        {selectedUser ? (
          <div className="flex flex-wrap items-center gap-3">
            <div>
              <div className="font-medium text-on-surface">
                {selectedUser.display_name || 'Unnamed User'}
              </div>
              <div className="text-sm text-on-surface-variant">{selectedUser.email}</div>
            </div>
            <div className="ml-auto">
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  setSelectedUser(null);
                  setToolSearch('');
                }}
              >
                Change user
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            <h3 className="font-medium text-on-surface">Select a user</h3>
            <Input
              type="search"
              placeholder="Search users by email or name..."
              aria-label="Search users"
              value={userSearch}
              onChange={(e) => setUserSearch(e.target.value)}
            />
            {usersLoading ? (
              <div className="flex items-center justify-center py-6">
                <div className="pierre-spinner w-6 h-6" />
              </div>
            ) : matchingUsers.length > 0 ? (
              <ul className="divide-y ghost-border">
                {matchingUsers.map((user) => (
                  <li key={user.id}>
                    <button
                      onClick={() => setSelectedUser(user)}
                      className="w-full flex items-center justify-between gap-3 py-2 px-2 text-left rounded hover:bg-surface-container-low transition-colors"
                    >
                      <span>
                        <span className="block text-sm font-medium text-on-surface">
                          {user.display_name || 'Unnamed User'}
                        </span>
                        <span className="block text-xs text-on-surface-variant">{user.email}</span>
                      </span>
                      <span className="text-xs text-outline capitalize">{user.tier}</span>
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-sm text-on-surface-variant py-2">No users match your search.</p>
            )}
          </div>
        )}
      </Card>

      {/* Tools for the selected user */}
      {selectedUser && (
        <>
          <div className="relative">
            <Input
              type="search"
              placeholder="Search tools by name, description..."
              aria-label="Search tools"
              value={toolSearch}
              onChange={(e) => setToolSearch(e.target.value)}
            />
          </div>

          {(setOverrideMutation.isError || removeOverrideMutation.isError) && (
            <div className="p-3 bg-error/20 text-error rounded-lg text-sm">
              Failed to update the tool override. Please try again.
            </div>
          )}

          {toolsLoading ? (
            <div className="flex items-center justify-center py-12">
              <div className="pierre-spinner w-8 h-8" />
            </div>
          ) : toolsError ? (
            <Card variant="dark" className="border-error/30">
              <div className="text-center py-8">
                <p className="text-error">Failed to load tools for this user.</p>
                <p className="text-sm text-on-surface-variant mt-2">
                  Please check your permissions and try again.
                </p>
              </div>
            </Card>
          ) : (
            <>
              {toolsByCategory.map(([category, tools]) => (
                <Card key={category} variant="dark">
                  <h3 className="font-medium text-on-surface capitalize mb-3">{category}</h3>
                  <ul className="divide-y ghost-border">
                    {tools.map((tool) => {
                      const globallyDisabled = tool.source === 'global_disabled';
                      return (
                        <li
                          key={tool.tool_name}
                          className={`flex items-center gap-3 py-3 ${globallyDisabled ? 'opacity-60' : ''}`}
                        >
                          <div className="min-w-0 flex-1">
                            <div className="font-medium text-on-surface">{tool.display_name}</div>
                            <div className="text-xs text-on-surface-variant mt-0.5">
                              <code>{tool.tool_name}</code>
                            </div>
                            <div
                              className="text-xs text-outline mt-1 max-w-md truncate"
                              title={tool.description}
                            >
                              {tool.description}
                            </div>
                          </div>

                          {sourceBadge(tool.source)}

                          {/* Toggle switch */}
                          <button
                            onClick={() => handleToggleTool(tool)}
                            disabled={globallyDisabled || setOverrideMutation.isPending}
                            className={`relative inline-flex h-6 w-11 flex-shrink-0 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2 ${
                              tool.is_enabled ? 'bg-success' : 'bg-outline'
                            } ${globallyDisabled ? 'opacity-50 cursor-not-allowed' : ''}`}
                            role="switch"
                            aria-checked={tool.is_enabled}
                            aria-label={`Toggle ${tool.display_name}`}
                            title={
                              globallyDisabled
                                ? 'Cannot toggle globally disabled tools'
                                : undefined
                            }
                          >
                            <span
                              className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                                tool.is_enabled ? 'translate-x-6' : 'translate-x-1'
                              }`}
                            />
                          </button>

                          {/* Reset affordance — only when a per-user override is in effect */}
                          {tool.source === 'user_override' && (
                            <button
                              onClick={() => removeOverrideMutation.mutate(tool.tool_name)}
                              disabled={removeOverrideMutation.isPending}
                              className="p-1 text-on-surface-variant hover:text-on-surface transition-colors"
                              title="Remove override (revert to plan/tenant/default)"
                            >
                              <svg
                                className="w-4 h-4"
                                fill="none"
                                stroke="currentColor"
                                viewBox="0 0 24 24"
                                aria-hidden="true"
                              >
                                <path
                                  strokeLinecap="round"
                                  strokeLinejoin="round"
                                  strokeWidth={2}
                                  d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                                />
                              </svg>
                            </button>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                </Card>
              ))}

              {toolsByCategory.length === 0 && (
                <Card variant="dark">
                  <div className="text-center py-8 text-on-surface-variant">
                    No tools found matching your criteria.
                  </div>
                </Card>
              )}
            </>
          )}
        </>
      )}
    </div>
  );
}
