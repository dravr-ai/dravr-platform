// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { a2aApi } from '../services/api';
import type { A2AClientRegistrationRequest, A2AClientCredentials } from '../types/api';
import { Button, Card, Textarea, Input } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import { useTranslation } from '@pierre/i18n';

interface CreateA2AClientProps {
  onSuccess?: () => void;
  onCancel?: () => void;
}

const AVAILABLE_CAPABILITIES = [
  {
    id: 'fitness-data-analysis',
    nameKey: 'a2a.capFitnessName',
    descriptionKey: 'a2a.capFitnessDesc'
  },
  {
    id: 'activity-intelligence',
    nameKey: 'a2a.capActivityName',
    descriptionKey: 'a2a.capActivityDesc'
  },
  {
    id: 'goal-management',
    nameKey: 'a2a.capGoalName',
    descriptionKey: 'a2a.capGoalDesc'
  },
  {
    id: 'performance-prediction',
    nameKey: 'a2a.capPredictionName',
    descriptionKey: 'a2a.capPredictionDesc'
  },
  {
    id: 'training-analytics',
    nameKey: 'a2a.capTrainingName',
    descriptionKey: 'a2a.capTrainingDesc'
  },
  {
    id: 'provider-integration',
    nameKey: 'a2a.capProviderName',
    descriptionKey: 'a2a.capProviderDesc'
  }
];

export default function CreateA2AClient({ onSuccess, onCancel }: CreateA2AClientProps) {
  const { t } = useTranslation();
  const [formData, setFormData] = useState<A2AClientRegistrationRequest>({
    name: '',
    description: '',
    capabilities: [],
    redirect_uris: [],
    contact_email: '',
    agent_version: '',
    documentation_url: ''
  });
  
  const [redirectUri, setRedirectUri] = useState('');
  const [showCredentials, setShowCredentials] = useState(false);
  const [credentials, setCredentials] = useState<A2AClientCredentials | null>(null);
  const [validationError, setValidationError] = useState<string | null>(null);
  
  const queryClient = useQueryClient();

  const createMutation = useMutation({
    mutationFn: (data: A2AClientRegistrationRequest) => a2aApi.registerA2AClient(data),
    onSuccess: (response: A2AClientCredentials) => {
      setCredentials(response);
      setShowCredentials(true);
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.a2a.clients() });
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    
    // Clear previous validation error
    setValidationError(null);
    
    if (formData.capabilities.length === 0) {
      setValidationError(t('a2a.selectCapability'));
      return;
    }

    createMutation.mutate(formData);
  };

  const handleCapabilityToggle = (capabilityId: string) => {
    setFormData(prev => ({
      ...prev,
      capabilities: prev.capabilities.includes(capabilityId)
        ? prev.capabilities.filter(id => id !== capabilityId)
        : [...prev.capabilities, capabilityId]
    }));
  };

  const handleAddRedirectUri = () => {
    if (redirectUri.trim() && !formData.redirect_uris?.includes(redirectUri.trim())) {
      setFormData(prev => ({
        ...prev,
        redirect_uris: [...(prev.redirect_uris || []), redirectUri.trim()]
      }));
      setRedirectUri('');
    }
  };

  const handleRemoveRedirectUri = (uri: string) => {
    setFormData(prev => ({
      ...prev,
      redirect_uris: prev.redirect_uris?.filter(u => u !== uri) || []
    }));
  };

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text);
    // You might want to show a toast notification here
  };

  const handleDone = () => {
    setShowCredentials(false);
    setCredentials(null);
    setFormData({
      name: '',
      description: '',
      capabilities: [],
      redirect_uris: [],
      contact_email: '',
      agent_version: '',
      documentation_url: ''
    });
    onSuccess?.();
  };

  if (showCredentials && credentials) {
    return (
      <Card variant="dark">
        <div className="text-center">
          <h2 className="text-2xl font-bold text-on-surface mb-2">{t('a2a.createdTitle')}</h2>
          <p className="text-on-surface-variant mb-6">
            {t('a2a.createdBody')}
          </p>
        </div>

        <div className="bg-nutrition/15 border border-nutrition/30 rounded-lg p-4 mb-6">
          <div className="flex items-center mb-2">
            <h3 className="text-sm font-medium text-nutrition">{t('a2a.securityNoticeTitle')}</h3>
          </div>
          <p className="text-sm text-on-surface">
            {t('a2a.securityNoticeBody')}
          </p>
        </div>

        <div className="space-y-4">
          {/* Client ID */}
          <div>
            <label className="block text-sm font-medium text-on-surface mb-2">
              {t('a2a.clientId')}
            </label>
            <div className="flex items-center gap-2">
              <code className="flex-1 bg-surface-container-high p-3 rounded font-mono text-sm break-all text-on-surface border ghost-border">
                {credentials.client_id}
              </code>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => copyToClipboard(credentials.client_id)}
              >
                {t('common.copy')}
              </Button>
            </div>
          </div>

          {/* Client Secret */}
          <div>
            <label className="block text-sm font-medium text-on-surface mb-2">
              {t('a2a.clientSecret')}
            </label>
            <div className="flex items-center gap-2">
              <code className="flex-1 bg-surface-container-high p-3 rounded font-mono text-sm break-all text-on-surface border ghost-border">
                {credentials.client_secret}
              </code>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => copyToClipboard(credentials.client_secret)}
              >
                {t('common.copy')}
              </Button>
            </div>
          </div>

          {/* API Key */}
          <div>
            <label className="block text-sm font-medium text-on-surface mb-2">
              {t('a2a.apiKey')}
            </label>
            <div className="flex items-center gap-2">
              <code className="flex-1 bg-surface-container-high p-3 rounded font-mono text-sm break-all text-on-surface border ghost-border">
                {credentials.api_key}
              </code>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => copyToClipboard(credentials.api_key)}
              >
                {t('common.copy')}
              </Button>
            </div>
          </div>
        </div>

        <div className="mt-8 text-center">
          <Button onClick={handleDone}>
            {t('common.done')}
          </Button>
        </div>
      </Card>
    );
  }

  return (
    <Card variant="dark">
      <form onSubmit={handleSubmit} className="space-y-6">
        <div>
          <h2 className="text-xl font-semibold text-on-surface mb-2">{t('a2a.registerTitle')}</h2>
          <p className="text-on-surface-variant">
            {t('a2a.registerBlurb')}
          </p>
        </div>

        {/* Basic Information */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <Input
            type="text"
            id="name"
            label={t('a2a.clientNameRequired')}
            value={formData.name}
            onChange={(e) => setFormData(prev => ({ ...prev, name: e.target.value }))}
            placeholder={t('a2a.clientNamePlaceholder')}
            required
          />

          <Input
            type="email"
            id="contact_email"
            label={t('a2a.contactEmailRequired')}
            value={formData.contact_email}
            onChange={(e) => setFormData(prev => ({ ...prev, contact_email: e.target.value }))}
            placeholder="contact@example.com"
            required
          />
        </div>

        <Textarea
          id="description"
          label={t('a2a.descriptionRequired')}
          value={formData.description}
          onChange={(e) => setFormData(prev => ({ ...prev, description: e.target.value }))}
          rows={3}
          placeholder={t('a2a.descriptionPlaceholder')}
          required
        />

        {/* Optional Fields */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <Input
            type="text"
            id="agent_version"
            label={t('a2a.agentVersion')}
            value={formData.agent_version}
            onChange={(e) => setFormData(prev => ({ ...prev, agent_version: e.target.value }))}
            placeholder="1.0.0"
          />

          <Input
            type="url"
            id="documentation_url"
            label={t('a2a.documentationUrl')}
            value={formData.documentation_url}
            onChange={(e) => setFormData(prev => ({ ...prev, documentation_url: e.target.value }))}
            placeholder="https://docs.example.com"
          />
        </div>

        {/* Capabilities */}
        <div>
          <label className="block text-sm font-medium text-on-surface mb-2">
            {t('a2a.capabilitiesLabel')}
          </label>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            {AVAILABLE_CAPABILITIES.map((capability) => (
              <div
                key={capability.id}
                className={`border rounded-lg p-3 cursor-pointer transition-colors ${
                  formData.capabilities.includes(capability.id)
                    ? 'border-primary bg-primary/10'
                    : 'ghost-border hover:ghost-border'
                }`}
                onClick={() => handleCapabilityToggle(capability.id)}
              >
                <div className="flex items-center">
                  <input
                    type="checkbox"
                    checked={formData.capabilities.includes(capability.id)}
                    onChange={() => handleCapabilityToggle(capability.id)}
                    className="mr-3 rounded ghost-border bg-surface-container-high text-primary focus:ring-primary"
                    onClick={(e) => e.stopPropagation()}
                  />
                  <div>
                    <h4 className="font-medium text-on-surface">{t(capability.nameKey)}</h4>
                    <p className="text-sm text-on-surface-variant">{t(capability.descriptionKey)}</p>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* Redirect URIs */}
        <div>
          <label className="block text-sm font-medium text-on-surface mb-2">
            {t('a2a.redirectUrisOptional')}
          </label>
          <div className="space-y-2">
            <div className="flex items-end gap-2">
              <Input
                type="url"
                value={redirectUri}
                onChange={(e) => setRedirectUri(e.target.value)}
                placeholder="https://example.com/callback"
              />
              <Button
                type="button"
                variant="secondary"
                onClick={handleAddRedirectUri}
                disabled={!redirectUri.trim()}
              >
                {t('common.add')}
              </Button>
            </div>
            {formData.redirect_uris && formData.redirect_uris.length > 0 && (
              <div className="space-y-1">
                {formData.redirect_uris.map((uri) => (
                  <div key={uri} className="flex items-center justify-between bg-surface-container-low p-2 rounded border ghost-border">
                    <code className="text-sm text-on-surface">{uri}</code>
                    <Button
                      type="button"
                      variant="danger"
                      size="sm"
                      onClick={() => handleRemoveRedirectUri(uri)}
                    >
                      {t('app.remove')}
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>

        {/* Error Display */}
        {createMutation.error && (
          <div className="bg-error/15 border border-error/30 rounded-lg p-4">
            <div className="flex items-center">
              <div>
                <h3 className="text-sm font-medium text-error">{t('a2a.registrationFailed')}</h3>
                <p className="text-sm text-on-surface mt-1">
                  {createMutation.error instanceof Error
                    ? createMutation.error.message
                    : t('a2a.registerError')}
                </p>
              </div>
            </div>
          </div>
        )}

        {/* Validation Error */}
        {validationError && (
          <div className="bg-error/15 border border-error/30 text-error px-4 py-3 rounded">
            {validationError}
          </div>
        )}

        {/* Actions */}
        <div className="flex gap-3 pt-4">
          <Button
            type="submit"
            disabled={createMutation.isPending || formData.capabilities.length === 0}
          >
            {createMutation.isPending ? t('a2a.creating') : t('a2a.registerClient')}
          </Button>
          <Button
            type="button"
            variant="secondary"
            onClick={onCancel}
            disabled={createMutation.isPending}
          >
            {t('common.cancel')}
          </Button>
        </div>
      </form>
    </Card>
  );
}