// ABOUTME: Messaging channel configuration tab for user settings
// ABOUTME: Allows admins to configure WhatsApp, Telegram, Slack, Discord, and Messenger channels
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { messagingApi } from '../services/api';
import type { ChannelConfigSummary } from '../services/api/messaging';
import { Card, Button, Input, Badge, ConfirmDialog } from './ui';
import { clsx } from 'clsx';
import { QUERY_KEYS } from '../constants/queryKeys';

interface ChannelField {
  key: string;
  label: string;
  type: 'text' | 'password';
  placeholder: string;
}

interface ChannelInfo {
  name: string;
  description: string;
  icon: React.ReactNode;
  fields: ChannelField[];
}

const CHANNEL_INFO: Record<string, ChannelInfo> = {
  whatsapp: {
    name: 'WhatsApp',
    description: 'WhatsApp Business Cloud API via Meta Graph API',
    icon: (
      <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 5a2 2 0 012-2h3.28a1 1 0 01.948.684l1.498 4.493a1 1 0 01-.502 1.21l-2.257 1.13a11.042 11.042 0 005.516 5.516l1.13-2.257a1 1 0 011.21-.502l4.493 1.498a1 1 0 01.684.949V19a2 2 0 01-2 2h-1C9.716 21 3 14.284 3 6V5z" />
      </svg>
    ),
    fields: [
      { key: 'api_key', label: 'Access Token', type: 'password', placeholder: 'WhatsApp Cloud API access token' },
      { key: 'webhook_secret', label: 'App Secret (HMAC)', type: 'password', placeholder: 'Meta App Secret for webhook signature verification' },
      { key: 'verify_token', label: 'Verify Token', type: 'text', placeholder: 'Token for Meta webhook URL verification' },
      { key: 'phone_number', label: 'Phone Number', type: 'text', placeholder: '+1 555 123 4567' },
    ],
  },
  telegram: {
    name: 'Telegram',
    description: 'Telegram Bot API for messaging and channel linking',
    icon: (
      <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
      </svg>
    ),
    fields: [
      { key: 'bot_token', label: 'Bot Token', type: 'password', placeholder: '123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11' },
      { key: 'webhook_secret', label: 'Webhook Secret', type: 'password', placeholder: 'Secret token for X-Telegram-Bot-Api-Secret-Token header' },
    ],
  },
  slack: {
    name: 'Slack',
    description: 'Slack Events API for workspace messaging',
    icon: (
      <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 8h10M7 12h4m1 8l-4-4H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-3l-4 4z" />
      </svg>
    ),
    fields: [
      { key: 'api_key', label: 'Bot Token', type: 'password', placeholder: 'xoxb-...' },
      { key: 'api_secret', label: 'Client Secret', type: 'password', placeholder: 'Slack app client secret' },
      { key: 'webhook_secret', label: 'Signing Secret', type: 'password', placeholder: 'Slack signing secret for request verification' },
    ],
  },
  discord: {
    name: 'Discord',
    description: 'Discord Bot API for server messaging',
    icon: (
      <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" />
      </svg>
    ),
    fields: [
      { key: 'bot_token', label: 'Bot Token', type: 'password', placeholder: 'Discord bot token' },
      { key: 'webhook_secret', label: 'Public Key', type: 'password', placeholder: 'Ed25519 public key (hex) for interaction verification' },
      { key: 'account_id', label: 'Application ID', type: 'text', placeholder: 'Discord application ID' },
    ],
  },
  messenger: {
    name: 'Messenger',
    description: 'Meta Messenger Platform for page messaging',
    icon: (
      <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
      </svg>
    ),
    fields: [
      { key: 'api_key', label: 'Page Access Token', type: 'password', placeholder: 'Messenger page access token' },
      { key: 'webhook_secret', label: 'App Secret (HMAC)', type: 'password', placeholder: 'Meta App Secret for webhook signature verification' },
      { key: 'verify_token', label: 'Verify Token', type: 'text', placeholder: 'Token for Meta webhook URL verification' },
    ],
  },
};

const CHANNEL_ORDER = ['whatsapp', 'telegram', 'slack', 'discord', 'messenger'];

export default function MessagingSettingsTab() {
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
      setMessage({ type: 'success', text: 'Channel configuration saved' });
      resetForm();
    },
    onError: (error: Error) => {
      setMessage({ type: 'error', text: error.message || 'Failed to save configuration' });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (channel: string) => messagingApi.deleteChannel(channel),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.messaging.channels() });
      setMessage({ type: 'success', text: 'Channel configuration removed' });
      setChannelToDelete(null);
    },
    onError: (error: Error) => {
      setMessage({ type: 'error', text: error.message || 'Failed to remove configuration' });
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
      <Card variant="dark">
        <div className="animate-pulse space-y-4">
          <div className="h-6 bg-white/10 rounded w-1/3"></div>
          <div className="h-20 bg-white/5 rounded"></div>
          <div className="h-20 bg-white/5 rounded"></div>
        </div>
      </Card>
    );
  }

  return (
    <>
      <Card variant="dark">
        <h2 className="text-lg font-semibold text-white mb-4">Messaging Channels</h2>
        <p className="text-sm text-white/60 mb-6">
          Configure messaging channels to enable users to chat with Pierre through WhatsApp,
          Telegram, Slack, Discord, or Messenger.
        </p>

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
                    ? 'border-pierre-activity/30 bg-pierre-activity-light/10'
                    : 'border-white/10 bg-white/5 hover:border-white/20'
                )}
              >
                <div className="flex items-start justify-between">
                  <div className="flex items-start gap-3 flex-1">
                    <div
                      className={clsx(
                        'p-2 rounded-lg',
                        isConfigured ? 'bg-pierre-activity/20 text-pierre-activity' : 'bg-white/10 text-white/60'
                      )}
                    >
                      {info.icon}
                    </div>
                    <div className="flex-1">
                      <div className="flex items-center gap-2 mb-1">
                        <h3 className="font-medium text-white">{info.name}</h3>
                        {isConfigured && <Badge variant="success">Configured</Badge>}
                      </div>
                      <p className="text-sm text-white/60">{info.description}</p>
                    </div>
                  </div>
                  <div className="flex gap-2 ml-4">
                    {isConfigured && (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => setChannelToDelete(channelId)}
                      >
                        Remove
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
                      {isConfigured ? 'Update' : 'Configure'}
                    </Button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </Card>

      {/* Configuration Form */}
      {selectedChannel && CHANNEL_INFO[selectedChannel] && (
        <Card variant="dark">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-semibold text-white">
              Configure {CHANNEL_INFO[selectedChannel].name}
            </h2>
            <button onClick={resetForm} className="text-white/40 hover:text-white/70">
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>

          <div className="space-y-4">
            {CHANNEL_INFO[selectedChannel].fields.map((field) => (
              <Input
                key={field.key}
                label={field.label}
                type={field.type}
                value={formValues[field.key] || ''}
                onChange={(e) =>
                  setFormValues((prev) => ({ ...prev, [field.key]: e.target.value }))
                }
                placeholder={field.placeholder}
              />
            ))}

            <div className="flex gap-3 pt-2">
              <Button variant="outline" onClick={resetForm}>
                Cancel
              </Button>
              <Button
                variant="gradient"
                onClick={handleSave}
                loading={saveMutation.isPending}
                disabled={Object.values(formValues).every((v) => !v.trim())}
              >
                Save Configuration
              </Button>
            </div>
          </div>
        </Card>
      )}

      {/* Status Message */}
      {message && (
        <Card variant="dark">
          <div
            className={clsx(
              'p-3 rounded-lg text-sm',
              message.type === 'success'
                ? 'bg-pierre-activity-light/30 text-pierre-activity'
                : 'bg-pierre-red-50 text-pierre-red-600'
            )}
          >
            {message.text}
          </div>
        </Card>
      )}

      {/* Delete Confirmation */}
      <ConfirmDialog
        isOpen={channelToDelete !== null}
        onClose={() => setChannelToDelete(null)}
        onConfirm={() => channelToDelete && deleteMutation.mutate(channelToDelete)}
        title="Remove Channel Configuration"
        message={`Are you sure you want to remove the ${
          channelToDelete ? CHANNEL_INFO[channelToDelete]?.name : ''
        } configuration? Users will no longer be able to chat through this channel.`}
        confirmLabel="Remove"
        variant="danger"
        isLoading={deleteMutation.isPending}
      />
    </>
  );
}
