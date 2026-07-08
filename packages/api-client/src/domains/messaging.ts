// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Messaging domain API — end-user channel linking (Telegram/WhatsApp/Slack/…) for onboarding
// ABOUTME: getAvailableChannels / initLink / listLinks / deleteLink; cross-platform (web + mobile), not the admin config surface

import type { AxiosInstance } from 'axios';
import { ENDPOINTS } from '../core/endpoints';

/** A messaging channel the tenant has configured that a user can connect to. */
export interface AvailableChannel {
  /** Channel id (`telegram`, `whatsapp`, `slack`, `discord`, `messenger`). */
  channel: string;
  /** User-facing name. */
  display_name: string;
  /** Linking method: `deep_link` (QR / tap) or `oauth` (redirect). */
  method: string;
  /** Whether to pre-select this channel (Telegram is the most complete). */
  recommended: boolean;
}

/** Response from initiating a channel link. */
export interface LinkInitResponse {
  channel: string;
  /** `deep_link` or `oauth`. */
  method: string;
  /** Verification code (deep-link channels only; `null` for OAuth). */
  code: string | null;
  /** URL the user opens/sends (t.me/… , wa.me/… , or the OAuth callback). */
  linking_url: string;
  /** ISO expiry of the linking code. */
  expires_at: string;
  /** Inline SVG QR of `linking_url` (deep-link channels only; `null` otherwise). */
  qr_svg: string | null;
}

/** A channel the user has already linked. */
export interface ChannelLink {
  channel: string;
  channel_user_id: string;
  display_name: string | null;
  linked_at: string;
}

/**
 * Creates the messaging API methods bound to an axios instance.
 *
 * End-user channel linking for the onboarding flow. Distinct from the web-only
 * admin channel-config surface (`frontend/src/services/api/messaging.ts`): these
 * endpoints never expose credentials.
 */
export function createMessagingApi(axios: AxiosInstance) {
  return {
    /** Channels the tenant has configured that the user can connect (secret-free). */
    async getAvailableChannels(): Promise<AvailableChannel[]> {
      const response = await axios.get<AvailableChannel[]>(
        ENDPOINTS.MESSAGING.CHANNELS_AVAILABLE,
      );
      return response.data;
    },

    /** Start linking a channel: returns the linking URL/code (+ QR for deep-link). */
    async initLink(channel: string): Promise<LinkInitResponse> {
      const response = await axios.post<LinkInitResponse>(ENDPOINTS.MESSAGING.LINK_INIT(channel));
      return response.data;
    },

    /** The channels the user has already linked. Poll this to detect completion. */
    async listLinks(): Promise<ChannelLink[]> {
      const response = await axios.get<{ links: ChannelLink[] }>(ENDPOINTS.MESSAGING.LINKS);
      return response.data.links ?? [];
    },

    /** Unlink a channel. */
    async deleteLink(channel: string): Promise<void> {
      await axios.delete(ENDPOINTS.MESSAGING.LINK(channel));
    },
  };
}

/** The messaging API surface (end-user channel linking). */
export type MessagingApi = ReturnType<typeof createMessagingApi>;
