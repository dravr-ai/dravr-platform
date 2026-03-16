// ABOUTME: Web-only messaging channel configuration API service
// ABOUTME: CRUD operations for per-tenant messaging channel configs (WhatsApp, Telegram, etc.)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { pierreApi } from './client';

interface ChannelConfigResponse {
  tenant_id: string;
  channels: ChannelConfigSummary[];
}

export interface ChannelConfigSummary {
  id: string;
  tenant_id: string;
  channel_type: string;
  is_active: boolean;
  created_at: string;
  updated_at: string;
}

interface UpsertChannelConfigRequest {
  enabled: boolean;
  credentials: Record<string, string>;
  webhook_url?: string;
}

interface UpsertChannelConfigResponse {
  tenant_id: string;
  config: {
    channel: string;
    enabled: boolean;
    has_credentials: boolean;
    webhook_url: string | null;
  };
  action: string;
}

interface DeleteChannelConfigResponse {
  tenant_id: string;
  channel: string;
  action: string;
}

export const messagingApi = {
  async listChannels(): Promise<ChannelConfigResponse> {
    return pierreApi.adapter.get<ChannelConfigResponse>('/api/messaging/channels');
  },

  async upsertChannel(
    channel: string,
    body: UpsertChannelConfigRequest
  ): Promise<UpsertChannelConfigResponse> {
    return pierreApi.adapter.put<UpsertChannelConfigResponse>(
      `/api/messaging/channels/${channel}`,
      body
    );
  },

  async deleteChannel(channel: string): Promise<DeleteChannelConfigResponse> {
    return pierreApi.adapter.delete<DeleteChannelConfigResponse>(
      `/api/messaging/channels/${channel}`
    );
  },
};
