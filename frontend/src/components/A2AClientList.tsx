// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { a2aApi } from '../services/api';
import type { A2AClient, A2AUsageStats, A2ARateLimitStatus } from '../types/api';
import { Button, Section, Badge, StatusIndicator, StatusFilter, ConfirmDialog } from './ui';
import type { StatusFilterValue } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import { useTranslation } from '@pierre/i18n';
/**
 * How long ago `date` was, in the athlete's language.
 *
 * The hand-rolled version returned "3 days", "just now" and an English plural
 * rule, and its callers appended a bare "ago" — five English strings on a
 * settings pane an athlete sees whenever the API-tokens flag is on. The
 * platform's own relative-time formatting carries the wording, the plural and
 * the "ago" for every locale.
 */
const formatDistanceToNow = (date: Date, language: string): string => {
  const relative = new Intl.RelativeTimeFormat(language, { numeric: 'auto' });
  const elapsedMs = date.getTime() - Date.now();
  const minutes = Math.round(elapsedMs / 60_000);
  if (Math.abs(minutes) < 1) return relative.format(0, 'minute');
  const hours = Math.round(elapsedMs / 3_600_000);
  if (Math.abs(hours) < 1) return relative.format(minutes, 'minute');
  const days = Math.round(elapsedMs / 86_400_000);
  if (Math.abs(days) < 1) return relative.format(hours, 'hour');
  return relative.format(days, 'day');
};

const format = (date: Date, pattern: string) => {
  if (pattern === 'MMM d, yyyy') {
    return date.toLocaleDateString('en-US', { 
      month: 'short', 
      day: 'numeric', 
      year: 'numeric' 
    });
  }
  return date.toLocaleDateString();
};

interface A2AClientListProps {
  onCreateClient?: () => void;
}

export default function A2AClientList({ onCreateClient }: A2AClientListProps) {
  const { t, language } = useTranslation();
  const [selectedClient, setSelectedClient] = useState<string | null>(null);
  const [showCredentials, setShowCredentials] = useState<{ [key: string]: boolean }>({});
  const [statusFilter, setStatusFilter] = useState<StatusFilterValue>('active');
  const [clientToDeactivate, setClientToDeactivate] = useState<A2AClient | null>(null);
  const queryClient = useQueryClient();

  const { data: clients, isLoading, error } = useQuery<A2AClient[]>({
    queryKey: QUERY_KEYS.a2a.clients(),
    queryFn: () => a2aApi.getA2AClients(),
  });

  const { data: clientUsage } = useQuery<A2AUsageStats | null>({
    queryKey: QUERY_KEYS.a2a.clientUsage(selectedClient ?? undefined),
    queryFn: () => selectedClient ? a2aApi.getA2AClientUsage(selectedClient) : Promise.resolve(null),
    enabled: !!selectedClient,
  });

  const { data: clientRateLimit } = useQuery<A2ARateLimitStatus | null>({
    queryKey: QUERY_KEYS.a2a.clientRateLimit(selectedClient ?? undefined),
    queryFn: () => selectedClient ? a2aApi.getA2AClientRateLimit(selectedClient) : Promise.resolve(null),
    enabled: !!selectedClient,
  });

  const deactivateMutation = useMutation({
    mutationFn: (clientId: string) => a2aApi.deactivateA2AClient(clientId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.a2a.clients() });
      setSelectedClient(null);
      setClientToDeactivate(null);
    },
  });

  const allClients = useMemo(() => clients || [], [clients]);

  // Compute counts for the filter
  const activeCount = useMemo(() => allClients.filter(c => c.is_active).length, [allClients]);
  const inactiveCount = useMemo(() => allClients.filter(c => !c.is_active).length, [allClients]);

  // Filter clients based on status filter
  const filteredClients = useMemo(() => {
    switch (statusFilter) {
      case 'active':
        return allClients.filter(c => c.is_active);
      case 'inactive':
        return allClients.filter(c => !c.is_active);
      case 'all':
      default:
        return allClients;
    }
  }, [allClients, statusFilter]);

  const getTierBadgeColor = (tier: string) => {
    switch (tier.toLowerCase()) {
      case 'trial':
        return 'bg-nutrition/20 text-on-nutrition-container border border-nutrition/30';
      case 'standard':
        return 'bg-primary-container text-on-primary-container border border-primary/20';
      case 'professional':
        return 'bg-activity/20 text-on-activity-container border border-activity/30';
      case 'enterprise':
        return 'bg-primary/20 text-primary border border-primary/30';
      default:
        return 'bg-surface-container-high text-on-surface-variant border ghost-border';
    }
  };

  const getCapabilityBadgeColor = (capability: string) => {
    const colorMap: { [key: string]: string } = {
      'fitness-data-analysis': 'bg-primary-container text-on-primary-container border border-primary/20',
      'activity-intelligence': 'bg-activity/20 text-on-activity-container border border-activity/30',
      'goal-management': 'bg-primary/20 text-primary border border-primary/30',
      'performance-prediction': 'bg-nutrition/20 text-on-nutrition-container border border-nutrition/30',
      'training-analytics': 'bg-primary-container text-on-primary-container border border-primary/20',
      'provider-integration': 'bg-recovery/20 text-on-recovery-container border border-recovery/30',
    };
    return colorMap[capability] || 'bg-surface-container-high text-on-surface-variant border ghost-border';
  };

  const handleDeactivate = (client: A2AClient) => {
    setClientToDeactivate(client);
  };

  const confirmDeactivate = () => {
    if (clientToDeactivate) {
      deactivateMutation.mutate(clientToDeactivate.id);
    }
  };

  const toggleCredentials = (clientId: string) => {
    setShowCredentials(prev => ({
      ...prev,
      [clientId]: !prev[clientId]
    }));
  };

  if (isLoading) {
    return (
      <div className="animate-pulse">
        <div className="mb-3 h-3 w-1/4 rounded bg-surface-container-high"></div>
        <div className="space-y-2">
          <div className="h-10 rounded bg-surface-container-high"></div>
          <div className="h-10 rounded bg-surface-container-high"></div>
          <div className="h-10 rounded bg-surface-container-high"></div>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="py-3">
        <h3 className="font-sans text-sm font-medium tracking-normal text-error">{t('a2a.loadFailedTitle')}</h3>
        <p className="mt-0.5 text-xs text-on-surface-variant">{t('a2a.loadFailedBody')}</p>
        <Button onClick={() => window.location.reload()} variant="tertiary" size="sm" className="mt-1 px-0">
          {t('common.tryAgain')}
        </Button>
      </div>
    );
  }

  if (allClients.length === 0) {
    return (
      <div className="py-3">
        <h3 className="font-sans text-sm font-medium tracking-normal text-on-surface-variant">{t('a2a.emptyTitle')}</h3>
        <p className="mt-0.5 max-w-md text-xs text-outline">{t('a2a.emptyBody')}</p>
        <Button onClick={onCreateClient} variant="tertiary" size="sm" className="mt-1 px-0">
          {t('a2a.emptyCta')}
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* A2A Client List */}
      <Section title={t('a2a.yourConnectedApps')} description={t('a2a.totalAppsCount', { count: allClients.length })}>
        {/* Status Filter */}
        <div className="pb-3">
          <StatusFilter
            value={statusFilter}
            onChange={setStatusFilter}
            activeCount={activeCount}
            inactiveCount={inactiveCount}
            totalCount={allClients.length}
          />
        </div>

        <div>
          {filteredClients.map((client) => (
            <div
              key={client.id}
              className={`cursor-pointer border-t ghost-border-faint py-4 transition-colors first:border-t-0 ${
                selectedClient === client.id ? 'bg-surface-container-low' : 'hover:bg-surface-container-low/60'
              }`}
              onClick={() => setSelectedClient(selectedClient === client.id ? null : client.id)}
            >
              <div className="flex items-center justify-between">
                <div className="flex-1">
                  <div className="flex items-center gap-3 mb-2">
                    <h3 className="font-sans text-sm font-semibold tracking-normal text-on-surface">{client.name}</h3>
                    <StatusIndicator
                      status={client.is_active ? 'online' : 'offline'}
                      size="sm"
                    />
                    {client.is_verified && (
                      <Badge variant="success" className="bg-activity/20 text-on-activity-container border border-activity/30">
                        {t('a2a.verified')}
                      </Badge>
                    )}
                  </div>
                  <p className="text-on-surface-variant mb-3">{client.description}</p>
                  
                  {/* Capabilities */}
                  <div className="flex flex-wrap gap-2 mb-3">
                    {client.capabilities.map((capability) => (
                      <Badge
                        key={capability}
                        variant="info"
                        className={getCapabilityBadgeColor(capability)}
                      >
                        {capability}
                      </Badge>
                    ))}
                  </div>

                  <div className="flex items-center gap-4 text-sm text-outline">
                    <span>{t('a2a.createdAgo', { when: formatDistanceToNow(new Date(client.created_at), language) })}</span>
                    {client.agent_version && <span>v{client.agent_version}</span>}
                  </div>
                </div>

                <div className="flex items-center gap-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleCredentials(client.id);
                    }}
                  >
                    {showCredentials[client.id] ? t('a2a.hideCredentials') : t('a2a.showCredentials')}
                  </Button>
                  {client.is_active && (
                    <Button
                      variant="danger"
                      size="sm"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDeactivate(client);
                      }}
                      disabled={deactivateMutation.isPending}
                    >
                      {t('app.deactivate')}
                    </Button>
                  )}
                </div>
              </div>

              {/* Credentials (when expanded) */}
              {showCredentials[client.id] && (
                <div className="mt-4 pt-4 border-t ghost-border">
                  <h4 className="text-sm font-medium text-on-surface mb-2">{t('a2a.clientCredentials')}</h4>
                  <div className="space-y-2 text-sm">
                    <div>
                      <label className="text-on-surface-variant">{t('a2a.clientIdLabel')}</label>
                      <code className="block bg-surface-container-high p-2 rounded font-mono text-xs mt-1 text-on-surface">
                        {client.id}
                      </code>
                    </div>
                    <div className="text-outline text-xs">
                      {t('a2a.secretShownOnce')}
                    </div>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      </Section>

      {/* Client Details */}
      {selectedClient && clientUsage && clientRateLimit && (
        <Section title={t('a2a.usageAndLimits')}>

          <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
            {/* Usage Stats */}
            <div>
              <h4 className="text-sm font-medium text-on-surface mb-2">{t('a2a.usageStatistics')}</h4>
              <div className="space-y-2">
                <div className="flex justify-between">
                  <span className="text-on-surface-variant">{t('a2a.today')}</span>
                  <span className="font-medium text-on-surface">{clientUsage?.requests_today?.toLocaleString() || 0}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-on-surface-variant">{t('a2a.thisMonth')}</span>
                  <span className="font-medium text-on-surface">{clientUsage?.requests_this_month?.toLocaleString() || 0}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-on-surface-variant">{t('a2a.total')}</span>
                  <span className="font-medium text-on-surface">{clientUsage?.total_requests?.toLocaleString() || 0}</span>
                </div>
                {clientUsage?.last_request_at && (
                  <div className="flex justify-between">
                    <span className="text-on-surface-variant">{t('a2a.lastRequest')}</span>
                    <span className="font-medium text-on-surface">
                      {formatDistanceToNow(new Date(clientUsage.last_request_at), language)}
                    </span>
                  </div>
                )}
              </div>
            </div>

            {/* Rate Limits */}
            <div>
              <h4 className="text-sm font-medium text-on-surface mb-2">{t('a2a.rateLimits')}</h4>
              <div className="space-y-2">
                <div className="flex justify-between">
                  <span className="text-on-surface-variant">{t('a2a.tier')}</span>
                  <Badge variant="info" className={getTierBadgeColor(clientRateLimit?.tier || 'trial')}>
                    {clientRateLimit?.tier || t('a2a.tierTrial')}
                  </Badge>
                </div>
                {clientRateLimit?.limit && (
                  <>
                    <div className="flex justify-between">
                      <span className="text-on-surface-variant">{t('a2a.monthlyLimit')}</span>
                      <span className="font-medium text-on-surface">{clientRateLimit.limit.toLocaleString()}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-on-surface-variant">{t('a2a.remaining')}</span>
                      <span className={`font-medium ${
                        clientRateLimit.remaining && clientRateLimit.remaining < clientRateLimit.limit * 0.1
                          ? 'text-error'
                          : 'text-activity'
                      }`}>
                        {clientRateLimit.remaining?.toLocaleString() || 0}
                      </span>
                    </div>
                    {clientRateLimit.reset_at && (
                      <div className="flex justify-between">
                        <span className="text-on-surface-variant">{t('a2a.resets')}</span>
                        <span className="font-medium text-on-surface">
                          {format(new Date(clientRateLimit.reset_at), 'MMM d, yyyy')}
                        </span>
                      </div>
                    )}
                  </>
                )}
              </div>
            </div>

            {/* Tool Usage */}
            <div>
              <h4 className="text-sm font-medium text-on-surface mb-2">{t('a2a.topTools')}</h4>
              <div className="space-y-2">
                {clientUsage?.tool_usage_breakdown?.slice(0, 3).map((tool: { tool_name: string; usage_count: number }) => (
                  <div key={tool.tool_name} className="flex justify-between">
                    <span className="text-on-surface-variant truncate">{tool.tool_name}:</span>
                    <span className="font-medium text-on-surface">{tool.usage_count}</span>
                  </div>
                ))}
                {(!clientUsage?.tool_usage_breakdown || clientUsage.tool_usage_breakdown.length === 0) && (
                  <div className="text-outline text-sm">{t('a2a.noToolUsage')}</div>
                )}
              </div>
            </div>
          </div>
        </Section>
      )}

      {/* Deactivate Confirmation */}
      <ConfirmDialog
        isOpen={clientToDeactivate !== null}
        onClose={() => setClientToDeactivate(null)}
        onConfirm={confirmDeactivate}
        title={t('a2a.deactivateTitle')}
        message={t('a2a.confirmDeactivate', { name: clientToDeactivate?.name ?? '' })}
        confirmLabel={t('app.deactivate')}
        cancelLabel={t('common.cancel')}
        variant="danger"
        isLoading={deactivateMutation.isPending}
      />
    </div>
  );
}