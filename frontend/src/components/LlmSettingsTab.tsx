// ABOUTME: LLM provider settings tab for user configuration
// ABOUTME: Allows users to configure API keys for Gemini, Groq, and local LLM providers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { userApi } from '../services/api';
import { Card, Button, Input, Badge, ConfirmDialog } from './ui';
import { clsx } from 'clsx';
import { QUERY_KEYS } from '../constants/queryKeys';
import { useAuth } from '../hooks/useAuth';
import { useTranslation } from '@pierre/i18n';

const PROVIDER_INFO: Record<string, { descriptionKey: string; docsUrl: string }> = {
  gemini: {
    descriptionKey: 'llmTab.geminiDesc',
    docsUrl: 'https://ai.google.dev/docs',
  },
  groq: {
    descriptionKey: 'llmTab.groqDesc',
    docsUrl: 'https://console.groq.com/docs',
  },
  cohere: {
    descriptionKey: 'llmTab.cohereDesc',
    docsUrl: 'https://docs.cohere.com/docs/command-a',
  },
  local: {
    descriptionKey: 'llmTab.localDesc',
    docsUrl: 'https://github.com/ollama/ollama',
  },
};

export default function LlmSettingsTab() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const [apiKey, setApiKey] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [defaultModel, setDefaultModel] = useState('');
  const [isValidating, setIsValidating] = useState(false);
  const [validationResult, setValidationResult] = useState<{
    valid: boolean;
    models?: string[];
    error?: string;
  } | null>(null);
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const [providerToDelete, setProviderToDelete] = useState<string | null>(null);

  // Fetch current LLM settings
  const { data: settings, isLoading } = useQuery({
    queryKey: QUERY_KEYS.llmSettings.list(),
    queryFn: () => userApi.getLlmSettings(),
  });

  // Save credentials mutation
  const saveMutation = useMutation({
    mutationFn: (data: {
      provider: string;
      api_key: string;
      base_url?: string;
      default_model?: string;
    }) => userApi.saveLlmCredentials(data),
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.llmSettings.list() });
      setMessage({ type: 'success', text: data.message });
      resetForm();
    },
    onError: (error: Error) => {
      setMessage({ type: 'error', text: error.message || t('llmTab.saveFailed') });
    },
  });

  // Delete credentials mutation
  const deleteMutation = useMutation({
    mutationFn: (provider: string) => userApi.deleteLlmCredentials(provider),
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.llmSettings.list() });
      setMessage({ type: 'success', text: data.message });
      setProviderToDelete(null);
    },
    onError: (error: Error) => {
      setMessage({ type: 'error', text: error.message || t('llmTab.deleteFailed') });
      setProviderToDelete(null);
    },
  });

  const resetForm = () => {
    setSelectedProvider(null);
    setApiKey('');
    setBaseUrl('');
    setDefaultModel('');
    setValidationResult(null);
  };

  const handleValidate = async () => {
    if (!selectedProvider || !apiKey.trim()) return;

    setIsValidating(true);
    setValidationResult(null);

    try {
      const result = await userApi.validateLlmCredentials({
        provider: selectedProvider,
        api_key: apiKey.trim(),
        base_url: selectedProvider === 'local' ? baseUrl.trim() || undefined : undefined,
      });

      setValidationResult({
        valid: result.valid,
        models: (result as { models?: string[] }).models || undefined,
        error: result.error || undefined,
      });
    } catch (error) {
      setValidationResult({
        valid: false,
        error: error instanceof Error ? error.message : t('llmTab.validationFailed'),
      });
    } finally {
      setIsValidating(false);
    }
  };

  const handleSave = () => {
    if (!selectedProvider || !apiKey.trim()) return;

    saveMutation.mutate({
      provider: selectedProvider,
      api_key: apiKey.trim(),
      base_url: selectedProvider === 'local' ? baseUrl.trim() || undefined : undefined,
      default_model: defaultModel.trim() || undefined,
    });
  };

  const getSourceBadge = (source: string | null) => {
    switch (source) {
      case 'user_specific':
        return <Badge variant="info">{t('llmTab.sourceUserKey')}</Badge>;
      case 'tenant_default':
        return <Badge variant="secondary">{t('llmTab.sourceOrganization')}</Badge>;
      case 'environment':
        return <Badge variant="warning">{t('llmTab.sourceSystem')}</Badge>;
      default:
        return null;
    }
  };

  if (isLoading) {
    return (
      <Card variant="dark">
        <div className="animate-pulse space-y-4">
          <div className="h-6 bg-surface-container-high rounded w-1/3"></div>
          <div className="h-20 bg-surface-container-low rounded"></div>
          <div className="h-20 bg-surface-container-low rounded"></div>
        </div>
      </Card>
    );
  }

  const providers = settings?.providers || [];
  const currentProvider = settings?.current_provider;
  const systemProvider = settings?.system_provider;

  const SystemProviderBanner = ({ provider }: { provider: { name: string; display_name: string; model?: string } }) => (
    <div className="p-4 bg-activity/20 border border-activity/30 rounded-lg">
      <div className="flex items-center gap-2 mb-2">
        <svg
          className="w-5 h-5 text-activity"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"
          />
        </svg>
        <span className="text-sm font-medium text-activity">
          {t('llmTab.activeForChat', { provider: provider.display_name })}
        </span>
        <Badge variant="warning">{t('llmTab.sourceSystem')}</Badge>
      </div>
      {provider.model && (
        <p className="text-sm text-on-surface/60 ml-7">
          {t('llmTab.modelLabel')} <code className="text-on-surface/80">{provider.model}</code>
        </p>
      )}
      <p className="text-sm text-on-surface/60 ml-7 mt-1">
        {t('llmTab.providerLabel')} <code className="text-on-surface/80">{provider.name}</code>
      </p>
    </div>
  );

  // Admin users see the system provider as read-only (no per-user override slot)
  if (user?.is_admin && systemProvider) {
    return (
      <Card variant="dark">
        <h2 className="text-lg font-semibold text-on-surface mb-4">{t('llmTab.systemProviderTitle')}</h2>
        <p className="text-sm text-on-surface/60 mb-6">{t('llmTab.systemProviderBlurb')}</p>
        <SystemProviderBanner provider={systemProvider} />
      </Card>
    );
  }

  return (
    <>
      {/* Current Status */}
      <Card variant="dark">
        <h2 className="text-lg font-semibold text-on-surface mb-4">{t('llmTab.title')}</h2>
        <p className="text-sm text-on-surface/60 mb-6">{t('llmTab.routingBlurb')}</p>

        {/* System provider is the authoritative chat router today (PIERRE_LLM_PROVIDER) */}
        {systemProvider && (
          <div className="mb-6">
            <SystemProviderBanner provider={systemProvider} />
          </div>
        )}

        {/* Stored credential indicator — only meaningful when no system override is in effect */}
        {!systemProvider && currentProvider && (
          <div className="mb-6 p-4 bg-activity/20 border border-activity/30 rounded-lg">
            <div className="flex items-center gap-2">
              <svg
                className="w-5 h-5 text-activity"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
              <span className="text-sm font-medium text-activity">
                {t('llmTab.activeProvider')}{' '}
                {providers.find((p) => p.name === currentProvider)?.display_name || currentProvider}
              </span>
            </div>
          </div>
        )}

        {/* Provider List */}
        <div className="space-y-4">
          {providers.map((provider) => (
            <div
              key={provider.name}
              data-testid={`llm-provider-card-${provider.name}`}
              className={clsx(
                'p-4 rounded-lg border transition-all',
                provider.has_credentials
                  ? 'border-activity/30 bg-activity/10'
                  : 'ghost-border bg-surface-container-low hover:ghost-border'
              )}
            >
              <div className="flex items-start justify-between">
                <div className="flex-1">
                  <div className="flex items-center gap-2 mb-1">
                    <h3 className="font-medium text-on-surface">{provider.display_name}</h3>
                    {provider.has_credentials && getSourceBadge(provider.credential_source ?? null)}
                    {provider.name === currentProvider && (
                      <Badge variant="success">{t('common.active')}</Badge>
                    )}
                  </div>
                  <p className="text-sm text-on-surface/60">
                    {t(PROVIDER_INFO[provider.name]?.descriptionKey ?? '')}
                  </p>
                  <a
                    href={PROVIDER_INFO[provider.name]?.docsUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-sm text-primary hover:underline mt-1 inline-block"
                  >
                    {t('llmTab.documentation')}
                  </a>
                </div>
                <div className="flex gap-2 ml-4">
                  {provider.has_credentials && provider.credential_source === 'user_specific' && (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => setProviderToDelete(provider.name)}
                    >
                      {t('app.remove')}
                    </Button>
                  )}
                  <Button
                    variant={provider.has_credentials ? 'outline' : 'gradient'}
                    size="sm"
                    onClick={() => {
                      setSelectedProvider(provider.name);
                      setMessage(null);
                      setValidationResult(null);
                    }}
                  >
                    {provider.has_credentials ? t('common.update') : t('common.configure')}
                  </Button>
                </div>
              </div>
            </div>
          ))}
        </div>
      </Card>

      {/* Configuration Form */}
      {selectedProvider && (
        <Card variant="dark">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-semibold text-on-surface">
              {t('llmTab.configureProvider', {
                provider: providers.find((p) => p.name === selectedProvider)?.display_name ?? '',
              })}
            </h2>
            <button
              onClick={resetForm}
              className="text-on-surface/40 hover:text-on-surface/70"
            >
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M6 18L18 6M6 6l12 12"
                />
              </svg>
            </button>
          </div>

          <div className="space-y-4">
            <Input
              label={t('llmTab.apiKey')}
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={t('llmTab.enterApiKey', { provider: selectedProvider.toUpperCase() })}
            />

            {selectedProvider === 'local' && (
              <Input
                label={t('llmTab.baseUrl')}
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="http://localhost:11434/v1"
                helpText={t('llmTab.baseUrlHint')}
              />
            )}

            <Input
              label={t('llmTab.defaultModelOptional')}
              value={defaultModel}
              onChange={(e) => setDefaultModel(e.target.value)}
              placeholder={
                selectedProvider === 'gemini'
                  ? 'gemini-1.5-flash'
                  : selectedProvider === 'groq'
                    ? 'llama-3.3-70b-versatile'
                    : selectedProvider === 'cohere'
                      ? 'command-a-03-2025'
                      : 'qwen2.5:14b-instruct'
              }
              helpText={t('llmTab.defaultModelHint')}
            />

            {/* Validation Result */}
            {validationResult && (
              <div
                className={clsx(
                  'p-4 rounded-lg',
                  validationResult.valid
                    ? 'bg-activity/30 border border-activity/30'
                    : 'bg-error border border-error'
                )}
              >
                {validationResult.valid ? (
                  <div>
                    <div className="flex items-center gap-2 text-activity font-medium mb-2">
                      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"
                        />
                      </svg>
                      {t('llmTab.keyValid')}
                    </div>
                    {validationResult.models && validationResult.models.length > 0 && (
                      <div className="text-sm text-on-surface/60">
                        {t('llmTab.availableModels')} {validationResult.models.slice(0, 5).join(', ')}
                        {validationResult.models.length > 5 && ` (+${validationResult.models.length - 5} more)`}
                      </div>
                    )}
                  </div>
                ) : (
                  <div className="flex items-center gap-2 text-error">
                    <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                      />
                    </svg>
                    {validationResult.error || t('llmTab.invalidApiKey')}
                  </div>
                )}
              </div>
            )}

            {/* Action Buttons */}
            <div className="flex gap-3 pt-2">
              <Button
                variant="outline"
                onClick={handleValidate}
                loading={isValidating}
                disabled={!apiKey.trim()}
              >
                {t('llmTab.testConnection')}
              </Button>
              <Button
                variant="gradient"
                onClick={handleSave}
                loading={saveMutation.isPending}
                disabled={!apiKey.trim() || (validationResult !== null && !validationResult.valid)}
              >
                {t('llmTab.saveApiKey')}
              </Button>
            </div>
          </div>
        </Card>
      )}

      {/* Success/Error Message - displayed outside form so it persists after save */}
      {message && (
        <Card variant="dark">
          <div
            className={clsx(
              'p-3 rounded-lg text-sm',
              message.type === 'success'
                ? 'bg-activity/30 text-on-activity-container'
                : 'bg-error text-error'
            )}
          >
            {message.text}
          </div>
        </Card>
      )}

      {/* Delete Confirmation Dialog */}
      <ConfirmDialog
        isOpen={providerToDelete !== null}
        onClose={() => setProviderToDelete(null)}
        onConfirm={() => providerToDelete && deleteMutation.mutate(providerToDelete)}
        title={t('llmTab.removeApiKey')}
        message={t('llmTab.confirmRemoveKey', {
          provider: providerToDelete?.toUpperCase() ?? '',
        })}
        confirmLabel={t('app.remove')}
        variant="danger"
        isLoading={deleteMutation.isPending}
      />
    </>
  );
}
