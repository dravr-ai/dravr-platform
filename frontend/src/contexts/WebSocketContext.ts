// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { createContext } from 'react';

export type WebSocketMessage = {
  type: 'auth' | 'subscribe' | 'system_stats' | 'error' | 'success';
  token?: string;
  topics?: string[];
  total_requests_today?: number;
  total_requests_this_month?: number;
  active_connections?: number;
  message?: string;
};

export interface WebSocketContextType {
  isConnected: boolean;
  lastMessage: WebSocketMessage | null;
  sendMessage: (message: WebSocketMessage) => void;
  subscribe: (topics: string[]) => void;
  reconnect: () => void;
}

export const WebSocketContext = createContext<WebSocketContextType | undefined>(undefined);