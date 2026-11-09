// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Toast context and types for the notification system
// ABOUTME: Separated from component to enable React Fast Refresh

import { createContext } from 'react';

export type ToastType = 'success' | 'error' | 'warning' | 'info';

/** The one thing a toast offers to do about what it says, as a real button. */
export interface ToastAction {
  label: string;
  onClick: () => void;
}

export interface Toast {
  id: string;
  type: ToastType;
  title: string;
  message?: string;
  duration?: number;
  action?: ToastAction;
  /**
   * While a toast with this key is on screen, adding another with the same key
   * shows nothing new — one failure reported by many parallel requests reads
   * as one notice.
   */
  dedupeKey?: string;
}

export interface ToastContextType {
  toasts: Toast[];
  addToast: (toast: Omit<Toast, 'id'>) => void;
  removeToast: (id: string) => void;
}

export const ToastContext = createContext<ToastContextType | undefined>(undefined);
