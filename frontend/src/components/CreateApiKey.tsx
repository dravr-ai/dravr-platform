// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import type { AdminPermission, CreateAdminTokenResponse } from '../types/api';
import { Textarea, Input } from './ui';

interface CreateApiKeyProps {
  onBack: () => void;
  onTokenCreated: (response: CreateAdminTokenResponse) => void;
}

const PERMISSION_DESCRIPTIONS: Record<AdminPermission, { label: string; description: string; danger?: boolean }> = {
  provision_keys: {
    label: 'Provision API Keys',
    description: 'Create new API keys for users and applications',
  },
  revoke_keys: {
    label: 'Revoke API Keys',
    description: 'Revoke existing API keys',
    danger: true,
  },
  list_keys: {
    label: 'List API Keys',
    description: 'View and list existing API keys',
  },
  manage_admin_tokens: {
    label: 'Manage Admin Tokens',
    description: 'Create, update, and manage admin tokens',
    danger: true,
  },
  view_audit_logs: {
    label: 'View Audit Logs',
    description: 'Access audit logs and activity history',
  },
  super_admin: {
    label: 'System Administration',
    description: 'Full system administration access',
    danger: true,
  },
};

export default function CreateApiKey({ onBack, onTokenCreated }: CreateApiKeyProps) {
  const [serviceName, setServiceName] = useState('');
  const [serviceDescription, setServiceDescription] = useState('');
  const [selectedPermissions, setSelectedPermissions] = useState<Set<AdminPermission>>(new Set(['provision_keys']));
  const [isSuperAdmin, setIsSuperAdmin] = useState(false);
  const [expiresInDays, setExpiresInDays] = useState<number | null>(365);
  const [neverExpires, setNeverExpires] = useState(false);

  const createTokenMutation = useMutation({
    mutationFn: (data: {
      service_name: string;
      service_description?: string;
      permissions?: string[];
      is_super_admin?: boolean;
      expires_in_days?: number;
    }) => adminApi.createAdminToken(data),
    onSuccess: (response) => {
      onTokenCreated(response);
    },
  });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    
    if (!serviceName.trim()) {
      return;
    }

    const tokenData = {
      service_name: serviceName.trim(),
      service_description: serviceDescription.trim() || undefined,
      permissions: isSuperAdmin ? undefined : Array.from(selectedPermissions),
      is_super_admin: isSuperAdmin,
      expires_in_days: neverExpires ? 0 : expiresInDays || undefined,
    };

    createTokenMutation.mutate(tokenData);
  };

  const handlePermissionToggle = (permission: AdminPermission) => {
    const newPermissions = new Set(selectedPermissions);
    if (newPermissions.has(permission)) {
      newPermissions.delete(permission);
    } else {
      newPermissions.add(permission);
    }
    setSelectedPermissions(newPermissions);
  };

  const handleSuperAdminToggle = (checked: boolean) => {
    setIsSuperAdmin(checked);
    if (checked) {
      setNeverExpires(true);
      setExpiresInDays(null);
    }
  };

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center gap-4">
        <button onClick={onBack} className="btn-secondary">
          ← Back
        </button>
        <div>
          <h2 className="text-xl font-semibold text-on-surface">Create API Token</h2>
          <p className="text-sm text-on-surface-variant mt-1">
            Generate a new API token for MCP clients and programmatic access
          </p>
        </div>
      </div>

      {/* Form */}
      <div className="bg-surface-container-low/60 border ghost-border rounded-lg p-6">
        <form onSubmit={handleSubmit} className="space-y-6">
          {createTokenMutation.error && (
            <div className="bg-error/15 border border-error/30 text-error px-4 py-3 rounded">
              {createTokenMutation.error instanceof Error
                ? createTokenMutation.error.message
                : 'Failed to create API token'}
            </div>
          )}

          {/* Service Details */}
          <div className="space-y-4">
            <h3 className="text-lg font-medium text-on-surface">Service Details</h3>

            <Input
              id="serviceName"
              label="Service Name *"
              type="text"
              required
              placeholder="e.g., pierre_admin_service, api_gateway"
              value={serviceName}
              onChange={(e) => setServiceName(e.target.value)}
              helpText="A unique identifier for the service that will use this token"
            />

            <Textarea
              id="serviceDescription"
              label="Description"
              rows={3}
              placeholder="Brief description of the service and its purpose"
              value={serviceDescription}
              onChange={(e) => setServiceDescription(e.target.value)}
            />
          </div>

          {/* Admin Level */}
          <div className="space-y-4">
            <h3 className="text-lg font-medium text-on-surface">Admin Level</h3>

            <div className="space-y-3">
              <label className="flex items-start gap-3">
                <input
                  type="checkbox"
                  checked={isSuperAdmin}
                  onChange={(e) => handleSuperAdminToggle(e.target.checked)}
                  className="mt-1 rounded ghost-border bg-surface-container-high text-primary focus:ring-primary"
                />
                <div>
                  <div className="font-medium text-on-surface">Super Admin API Token</div>
                  <p className="text-sm text-on-surface-variant">
                    Grants all permissions and never expires. Use with extreme caution.
                  </p>
                </div>
              </label>

              {isSuperAdmin && (
                <div className="bg-error/15 border border-error/30 rounded-lg p-4">
                  <div className="flex items-start gap-3">
                    <svg className="w-6 h-6 text-error mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.732-.833-2.5 0L4.732 16.5c-.77.833.192 2.5 1.732 2.5z" />
                    </svg>
                    <div>
                      <h4 className="font-medium text-error">Danger Zone</h4>
                      <p className="text-sm text-on-surface mt-1">
                        Super admin API tokens have unrestricted access to all system operations.
                        Only create these for trusted, critical services.
                      </p>
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* Permissions */}
          {!isSuperAdmin && (
            <div className="space-y-4">
              <h3 className="text-lg font-medium text-on-surface">Permissions</h3>
              <p className="text-sm text-on-surface-variant">
                Select the specific permissions this token should have
              </p>

              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                {Object.entries(PERMISSION_DESCRIPTIONS).map(([permission, info]) => (
                  <label key={permission} className="flex items-start gap-3 p-3 border ghost-border rounded-lg hover:bg-surface-container-low cursor-pointer">
                    <input
                      type="checkbox"
                      checked={selectedPermissions.has(permission as AdminPermission)}
                      onChange={() => handlePermissionToggle(permission as AdminPermission)}
                      className="mt-1 rounded ghost-border bg-surface-container-high text-primary focus:ring-primary"
                    />
                    <div className="flex-1">
                      <div className={`font-medium ${info.danger ? 'text-error' : 'text-on-surface'}`}>
                        {info.label}
                        {info.danger && (
                          <span className="ml-2 text-xs bg-error/20 text-error px-1.5 py-0.5 rounded border border-error/30">
                            High Risk
                          </span>
                        )}
                      </div>
                      <p className="text-sm text-on-surface-variant mt-1">
                        {info.description}
                      </p>
                    </div>
                  </label>
                ))}
              </div>

              {selectedPermissions.size === 0 && (
                <div className="bg-nutrition/15 border border-nutrition/30 rounded-lg p-3">
                  <p className="text-sm text-nutrition">
                    At least one permission must be selected for the token to be useful.
                  </p>
                </div>
              )}
            </div>
          )}

          {/* Expiration */}
          {!isSuperAdmin && (
            <div className="space-y-4">
              <h3 className="text-lg font-medium text-on-surface">Expiration</h3>

              <div className="space-y-3">
                <label className="flex items-center gap-3 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={neverExpires}
                    onChange={(e) => {
                      setNeverExpires(e.target.checked);
                      if (e.target.checked) {
                        setExpiresInDays(null);
                      } else {
                        setExpiresInDays(365);
                      }
                    }}
                    className="rounded ghost-border bg-surface-container-high text-primary focus:ring-primary"
                  />
                  <span className="font-medium text-on-surface">Never expires</span>
                </label>

                {!neverExpires && (
                  <div className="w-48">
                    <Input
                      id="expiresInDays"
                      label="Expires in (days)"
                      type="number"
                      min="1"
                      max="3650"
                      value={expiresInDays || ''}
                      onChange={(e) => setExpiresInDays(e.target.value ? parseInt(e.target.value) : null)}
                      helpText="Recommended: 365 days (1 year) for production services"
                    />
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Actions */}
          <div className="flex items-center gap-3 pt-4 border-t ghost-border">
            <button
              type="submit"
              disabled={
                createTokenMutation.isPending || 
                !serviceName.trim() || 
                (!isSuperAdmin && selectedPermissions.size === 0)
              }
              className="btn-primary disabled:opacity-50"
            >
              {createTokenMutation.isPending ? 'Creating API Token...' : 'Create API Token'}
            </button>
            <button
              type="button"
              onClick={onBack}
              disabled={createTokenMutation.isPending}
              className="btn-secondary"
            >
              Cancel
            </button>
          </div>
        </form>
      </div>

      {/* Security Reminder */}
      <div className="bg-primary-container border border-primary/20 rounded-lg p-4">
        <h4 className="font-medium text-primary mb-2">Security Reminder</h4>
        <ul className="text-sm text-on-surface space-y-1">
          <li>• The API token will be shown only once after creation</li>
          <li>• Store the token securely in your environment</li>
          <li>• Never commit API tokens to version control</li>
          <li>• Use HTTPS when transmitting API tokens</li>
          <li>• Regularly rotate tokens for better security</li>
        </ul>
      </div>
    </div>
  );
}