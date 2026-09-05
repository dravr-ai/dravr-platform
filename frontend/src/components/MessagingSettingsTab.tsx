// ABOUTME: Messaging channel configuration tab for user settings
// ABOUTME: Athletes configure their own WhatsApp, Telegram, Slack, Discord and Messenger credentials
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { messagingApi } from '../services/api';
import type { ChannelConfigSummary } from '../services/api/messaging';
import { Section, Button, Input, Badge, ConfirmDialog } from './ui';
import { clsx } from 'clsx';
import { QUERY_KEYS } from '../constants/queryKeys';
import { useTranslation } from '@pierre/i18n';
import { CHANNEL_BRAND } from '@pierre/shared-constants';

interface ChannelField {
  key: string;
  labelKey: string;
  type: 'text' | 'password';
  /** Corpus key, when the placeholder is a prose hint. */
  placeholderKey?: string;
  /** Verbatim format sample, when it is not. `xoxb-...` reads the same in
   *  every locale, and a translator handed it has no way to know that. */
  placeholderSample?: string;
}

interface ChannelInfo {
  name: string;
  descriptionKey: string;
  icon: React.ReactNode;
  fields: ChannelField[];
}

const CHANNEL_INFO: Record<string, ChannelInfo> = {
  whatsapp: {
    name: CHANNEL_BRAND.whatsapp,
    descriptionKey: 'msgChan.whatsappDesc',
    icon: (
      <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 5a2 2 0 012-2h3.28a1 1 0 01.948.684l1.498 4.493a1 1 0 01-.502 1.21l-2.257 1.13a11.042 11.042 0 005.516 5.516l1.13-2.257a1 1 0 011.21-.502l4.493 1.498a1 1 0 01.684.949V19a2 2 0 01-2 2h-1C9.716 21 3 14.284 3 6V5z" />
      </svg>
    ),
    fields: [
      { key: 'api_key', labelKey: 'msgChan.accessToken', type: 'password', placeholderKey: 'msgChan.whatsappAccessTokenHint' },
      { key: 'webhook_secret', labelKey: 'msgChan.appSecretHmac', type: 'password', placeholderKey: 'msgChan.metaAppSecretHint' },
      { key: 'verify_token', labelKey: 'msgChan.verifyToken', type: 'text', placeholderKey: 'msgChan.verifyTokenHint' },
      { key: 'phone_number', labelKey: 'msgChan.phoneNumber', type: 'text', placeholderSample: '+1 555 123 4567' },
    ],
  },
  telegram: {
    name: CHANNEL_BRAND.telegram,
    descriptionKey: 'msgChan.telegramDesc',
    icon: (
      <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
      </svg>
    ),
    fields: [
      { key: 'bot_token', labelKey: 'msgChan.botToken', type: 'password', placeholderSample: '123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11' },
      { key: 'webhook_secret', labelKey: 'msgChan.webhookSecret', type: 'password', placeholderKey: 'msgChan.telegramWebhookSecretHint' },
    ],
  },
  slack: {
    name: CHANNEL_BRAND.slack,
    descriptionKey: 'msgChan.slackDesc',
    icon: (
      <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 8h10M7 12h4m1 8l-4-4H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-3l-4 4z" />
      </svg>
    ),
    fields: [
      { key: 'api_key', labelKey: 'msgChan.botToken', type: 'password', placeholderSample: 'xoxb-...' },
      { key: 'api_secret', labelKey: 'msgChan.clientSecret', type: 'password', placeholderKey: 'msgChan.slackClientSecretHint' },
      { key: 'webhook_secret', labelKey: 'msgChan.signingSecret', type: 'password', placeholderKey: 'msgChan.slackSigningSecretHint' },
    ],
  },
  discord: {
    name: CHANNEL_BRAND.discord,
    descriptionKey: 'msgChan.discordDesc',
    icon: (
      <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" />
      </svg>
    ),
    fields: [
      { key: 'bot_token', labelKey: 'msgChan.botToken', type: 'password', placeholderKey: 'msgChan.discordBotTokenHint' },
      { key: 'webhook_secret', labelKey: 'msgChan.publicKey', type: 'password', placeholderKey: 'msgChan.discordPublicKeyHint' },
      { key: 'account_id', labelKey: 'msgChan.applicationId', type: 'text', placeholderKey: 'msgChan.discordAppIdHint' },
    ],
  },
  messenger: {
    name: CHANNEL_BRAND.messenger,
    descriptionKey: 'msgChan.messengerDesc',
    icon: (
      <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
      </svg>
    ),
    fields: [
      { key: 'api_key', labelKey: 'msgChan.pageAccessToken', type: 'password', placeholderKey: 'msgChan.messengerPageTokenHint' },
      { key: 'webhook_secret', labelKey: 'msgChan.appSecretHmac', type: 'password', placeholderKey: 'msgChan.metaAppSecretHint' },
      { key: 'verify_token', labelKey: 'msgChan.verifyToken', type: 'text', placeholderKey: 'msgChan.verifyTokenHint' },
    ],
  },
};

const CHANNEL_ORDER = ['whatsapp', 'telegram', 'slack', 'discord', 'messenger'];

export default function MessagingSettingsTab() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [selectedChannel, setSelectedChannel] = useState<string | null>(null);
  const [formValues, setFormValues] = useState<Record<string, string>>({});
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const [channelToDelete, setChannelToDelete] = useState<string | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: QUERY_KEYS.messaging.channels(),
    queryFn: () => messagingApi.listChannels(),
  });

  const saveMutation = useMutation({
    mutationFn: (params: { channel: string; credentials: Record<string, string> }) =>
      messagingApi.upsertChannel(params.channel, {
        enabled: true,
        credentials: params.credentials,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.messaging.channels() });
      setMessage({ type: 'success', text: t('msgChan.savedToast') });
      resetForm();
    },
    onError: (error: Error) => {
      setMessage({ type: 'error', text: error.message || t('msgChan.saveFailed') });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (channel: string) => messagingApi.deleteChannel(channel),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.messaging.channels() });
      setMessage({ type: 'success', text: t('msgChan.removedToast') });
      setChannelToDelete(null);
    },
    onError: (error: Error) => {
      setMessage({ type: 'error', text: error.message || t('msgChan.removeFailed') });
      setChannelToDelete(null);
    },
  });

  const resetForm = () => {
    setSelectedChannel(null);
    setFormValues({});
  };

  const handleSave = () => {
    if (!selectedChannel) return;
    const nonEmpty: Record<string, string> = {};
    for (const [k, v] of Object.entries(formValues)) {
      if (v.trim()) {
        nonEmpty[k] = v.trim();
      }
    }
    saveMutation.mutate({ channel: selectedChannel, credentials: nonEmpty });
  };

  const configuredChannels = new Set(
    (data?.channels || []).map((c: ChannelConfigSummary) => c.channel_type)
  );

  if (isLoading) {
    return (
      <div className="animate-pulse space-y-4">
        <div className="h-4 bg-surface-container-high rounded w-1/3"></div>
        <div className="h-12 bg-surface-container-low rounded"></div>
        <div className="h-12 bg-surface-container-low rounded"></div>
      </div>
    );
  }

  return (
    <>
      <Section title={t('msgChan.title')} description={t('msgChan.intro')}>

        <div className="space-y-4">
          {CHANNEL_ORDER.map((channelId) => {
            const info = CHANNEL_INFO[channelId];
            const isConfigured = configuredChannels.has(channelId);

            return (
              <div
                key={channelId}
                className={clsx(
                  'p-4 rounded-lg border transition-all',
                  isConfigured
                    ? 'border-activity/30 bg-activity/10'
                    : 'ghost-border bg-surface-container-low hover:ghost-border'
                )}
              >
                <div className="flex items-start justify-between">
                  <div className="flex items-start gap-3 flex-1">
                    <div
                      className={clsx(
                        'p-2 rounded-lg',
                        isConfigured ? 'bg-activity/20 text-on-activity-container' : 'bg-surface-container-high text-on-surface/60'
                      )}
                    >
                      {info.icon}
                    </div>
                    <div className="flex-1">
                      <div className="flex items-center gap-2 mb-1">
                        <h3 className="font-medium text-on-surface">{info.name}</h3>
                        {isConfigured && <Badge variant="success">{t('msgChan.configured')}</Badge>}
                      </div>
                      <p className="text-sm text-on-surface/60">{t(info.descriptionKey)}</p>
                    </div>
                  </div>
                  <div className="flex gap-2 ml-4">
                    {isConfigured && (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => setChannelToDelete(channelId)}
                      >
                        {t('app.remove')}
                      </Button>
                    )}
                    <Button
                      variant={isConfigured ? 'outline' : 'gradient'}
                      size="sm"
                      onClick={() => {
                        setSelectedChannel(channelId);
                        setFormValues({});
                        setMessage(null);
                      }}
                    >
                      {isConfigured ? t('common.update') : t('common.configure')}
                    </Button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </Section>

      {/* Configuration Form */}
      {selectedChannel && CHANNEL_INFO[selectedChannel] && (
        <Section
          title={t('msgChan.configureChannel', { channel: CHANNEL_INFO[selectedChannel].name })}
          actions={
            <button onClick={resetForm} className="text-on-surface/40 hover:text-on-surface/70">
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          }
        >

          <div className="space-y-4">
            {CHANNEL_INFO[selectedChannel].fields.map((field) => (
              <Input
                key={field.key}
                label={t(field.labelKey)}
                type={field.type}
                value={formValues[field.key] || ''}
                onChange={(e) =>
                  setFormValues((prev) => ({ ...prev, [field.key]: e.target.value }))
                }
                placeholder={field.placeholderKey ? t(field.placeholderKey) : field.placeholderSample}
              />
            ))}

            <div className="flex gap-3 pt-2">
              <Button variant="outline" onClick={resetForm}>
                {t('common.cancel')}
              </Button>
              <Button
                variant="gradient"
                onClick={handleSave}
                loading={saveMutation.isPending}
                disabled={Object.values(formValues).every((v) => !v.trim())}
              >
                {t('msgChan.saveConfiguration')}
              </Button>
            </div>
          </div>
        </Section>
      )}

      {/* Status Message */}
      {message && (
        <div
          className={clsx(
            'p-3 rounded-lg text-sm',
            message.type === 'success'
              ? 'bg-activity/30 text-on-activity-container'
              : 'bg-error/20 text-error'
          )}
        >
          {message.text}
        </div>
      )}

      {/* Delete Confirmation */}
      <ConfirmDialog
        isOpen={channelToDelete !== null}
        onClose={() => setChannelToDelete(null)}
        onConfirm={() => channelToDelete && deleteMutation.mutate(channelToDelete)}
        title={t('msgChan.removeChannelConfig')}
        message={t('msgChan.confirmRemove', {
          channel: channelToDelete ? CHANNEL_INFO[channelToDelete]?.name : '',
        })}
        confirmLabel={t('app.remove')}
        variant="danger"
        isLoading={deleteMutation.isPending}
      />
    </>
  );
}
