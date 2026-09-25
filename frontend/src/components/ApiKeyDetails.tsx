// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { format } from 'date-fns';
import { Button, Card, CardHeader, Badge, Textarea } from './ui';
import { adminApi } from '../services/api';
import type { AdminToken } from '../types/api';
import { QUERY_KEYS } from '../constants/queryKeys';

interface ApiKeyDetailsProps {
  token: AdminToken;
  onBack: () => void;
  onTokenUpdated: () => void;
}

interface TokenSuccessModalProps {
  isOpen: boolean;
  onClose: () => void;
  newToken: string;
  tokenInfo: AdminToken;
}

const TokenSuccessModal: React.FC<TokenSuccessModalProps> = ({ 
  isOpen, 
  onClose, 
  newToken, 
  tokenInfo 
}) => {
  const [copied, setCopied] = useState(false);

  const copyToClipboard = async () => {
    try {
      await navigator.clipboard.writeText(newToken);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy token:', err);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black/70 flex items-center justify-center z-50">
      <Card variant="dark" className="max-w-2xl mx-4 w-full">
        <CardHeader
          title="API Token Rotated Successfully"
          subtitle="Your new API token is ready"
        />

        <div className="space-y-6">
          <div className="bg-nutrition/15 border border-nutrition/30 rounded-lg p-4">
            <div className="flex items-start gap-3">
              <svg className="w-6 h-6 text-nutrition mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.732-.833-2.5 0L4.732 16.5c-.77.833.192 2.5 1.732 2.5z" />
              </svg>
              <div>
                <h4 className="font-medium text-nutrition">Important Security Notice</h4>
                <p className="text-sm text-on-surface mt-1">
                  This is the only time the full API token will be displayed. Please copy it now and store it securely.
                  The old token has been invalidated and will no longer work.
                </p>
              </div>
            </div>
          </div>

          <div>
            <div className="relative">
              <Textarea
                label="New API Key"
                className="font-mono !text-xs"
                value={newToken}
                readOnly
                rows={8}
                onClick={(e) => e.currentTarget.select()}
              />
              <Button
                variant="secondary"
                size="sm"
                className="absolute top-2 right-2"
                onClick={copyToClipboard}
              >
                {copied ? (
                  <>
                    <svg className="w-4 h-4 mr-1" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                    </svg>
                    Copied!
                  </>
                ) : (
                  <>
                    <svg className="w-4 h-4 mr-1" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
                    </svg>
                    Copy
                  </>
                )}
              </Button>
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4 text-sm">
            <div>
              <span className="text-on-surface-variant">Service:</span>
              <span className="ml-2 font-medium text-on-surface">{tokenInfo.service_name}</span>
            </div>
            <div>
              <span className="text-on-surface-variant">Prefix:</span>
              <span className="ml-2 font-mono text-on-surface">{tokenInfo.token_prefix}...</span>
            </div>
          </div>

          <div className="flex gap-3 pt-4 border-t ghost-border">
            <Button onClick={onClose} className="flex-1">
              I've Saved the API Token Securely
            </Button>
          </div>
        </div>
      </Card>
    </div>
  );
};

export default function ApiKeyDetails({ token, onBack, onTokenUpdated }: ApiKeyDetailsProps) {
  const queryClient = useQueryClient();
  const [showRotateModal, setShowRotateModal] = useState(false);
  const [rotatedToken, setRotatedToken] = useState<string>('');

  const revokeTokenMutation = useMutation({
    mutationFn: () => adminApi.revokeAdminToken(token.id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminTokens.all });
      onTokenUpdated();
      onBack();
    },
  });

  const rotateTokenMutation = useMutation({
    mutationFn: () => adminApi.rotateAdminToken(token.id),
    onSuccess: (data) => {
      setRotatedToken(data.jwt_token);
      setShowRotateModal(true);
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminTokens.all });
      onTokenUpdated();
    },
  });

  const handleRevoke = () => {
    const confirmed = confirm(
      `Are you sure you want to revoke the API token for "${token.service_name}"? This action cannot be undone and will immediately disable all access using this token.`
    );
    if (confirmed) {
      revokeTokenMutation.mutate();
    }
  };

  const handleRotate = () => {
    const confirmed = confirm(
      `Are you sure you want to rotate the API token for "${token.service_name}"? The current token will be invalidated and a new one will be generated.`
    );
    if (confirmed) {
      rotateTokenMutation.mutate();
    }
  };

  const getStatusBadge = () => {
    if (!token.is_active) {
      return <Badge variant="error">Revoked</Badge>;
    }
    
    if (token.expires_at) {
      const expiresAt = new Date(token.expires_at);
      const now = new Date();
      const daysUntilExpiry = Math.ceil((expiresAt.getTime() - now.getTime()) / (1000 * 60 * 60 * 24));
      
      if (expiresAt < now) {
        return <Badge variant="error">Expired</Badge>;
      } else if (daysUntilExpiry <= 7) {
        return <Badge variant="warning">Expires Soon</Badge>;
      }
    }
    
    if (token.is_super_admin) {
      return <Badge variant="enterprise">Super Admin</Badge>;
    }
    
    return <Badge variant="success">Active</Badge>;
  };

  return (
    <div className="max-w-4xl mx-auto space-y-6">
      <TokenSuccessModal
        isOpen={showRotateModal}
        onClose={() => setShowRotateModal(false)}
        newToken={rotatedToken}
        tokenInfo={token}
      />

      {/* Header */}
      <Card variant="dark">
        <CardHeader
          title={token.service_name}
          subtitle={`API Token • ${token.token_prefix}...`}
        >
          <div className="flex gap-3">
            <Button variant="secondary" onClick={onBack}>
              <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 19l-7-7m0 0l7-7m-7 7h18" />
              </svg>
              Back
            </Button>
            {token.is_active && (
              <>
                <Button
                  variant="secondary"
                  onClick={handleRotate}
                  loading={rotateTokenMutation.isPending}
                >
                  Rotate Key
                </Button>
                <Button
                  variant="danger"
                  onClick={handleRevoke}
                  loading={revokeTokenMutation.isPending}
                >
                  Revoke Key
                </Button>
              </>
            )}
          </div>
        </CardHeader>

        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
          <div>
            <span className="text-sm text-on-surface-variant">Status</span>
            <div className="mt-1">{getStatusBadge()}</div>
          </div>
          <div>
            <span className="text-sm text-on-surface-variant">Usage Count</span>
            <div className="text-xl font-semibold text-on-surface mt-1">
              {/* Defensive despite `usage_count` being non-optional in
                  shared-types: the server omitted it entirely for months and
                  the required type is exactly why neither side caught it. An
                  unguarded deref here throws past this component to the root
                  ErrorBoundary and blanks the whole SPA. */}
              {(token.usage_count ?? 0).toLocaleString()}
            </div>
          </div>
          <div>
            <span className="text-sm text-on-surface-variant">Created</span>
            <div className="text-sm text-on-surface mt-1">
              {format(new Date(token.created_at), 'MMM d, yyyy')}
            </div>
          </div>
          <div>
            <span className="text-sm text-on-surface-variant">Last Used</span>
            <div className="text-sm text-on-surface mt-1">
              {token.last_used_at
                ? format(new Date(token.last_used_at), 'MMM d, yyyy')
                : 'Never'
              }
            </div>
          </div>
        </div>
      </Card>

      {/* API Token Information */}
      <Card variant="dark">
        <CardHeader title="API Token Information" />
        <div className="space-y-4">
          <div>
            <label className="block text-sm font-medium text-on-surface-variant mb-1">Service Name</label>
            <div className="text-sm text-on-surface">{token.service_name}</div>
          </div>

          {token.service_description && (
            <div>
              <label className="block text-sm font-medium text-on-surface-variant mb-1">Description</label>
              <div className="text-sm text-on-surface">{token.service_description}</div>
            </div>
          )}

          <div>
            <label className="block text-sm font-medium text-on-surface-variant mb-1">Key Prefix</label>
            <div className="text-sm font-mono text-on-surface">{token.token_prefix}...</div>
          </div>

          <div>
            <label className="block text-sm font-medium text-on-surface-variant mb-1">Permissions</label>
            <div className="flex flex-wrap gap-2">
              {token.is_super_admin ? (
                <Badge variant="enterprise">All Permissions (Super Admin)</Badge>
              ) : (
                (token.permissions ?? []).map(permission => (
                  <Badge key={permission} variant="info">
                    {permission.replace(/_/g, ' ')}
                  </Badge>
                ))
              )}
            </div>
          </div>

          {token.expires_at && (
            <div>
              <label className="block text-sm font-medium text-on-surface-variant mb-1">Expires</label>
              <div className="text-sm text-on-surface">
                {format(new Date(token.expires_at), 'MMM d, yyyy \'at\' h:mm a')}
              </div>
            </div>
          )}
        </div>
      </Card>
    </div>
  );
}