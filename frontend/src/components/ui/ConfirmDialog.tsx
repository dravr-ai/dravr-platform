// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reusable confirmation dialog for destructive actions
// ABOUTME: Replaces native confirm() with consistent Pierre-styled modal

import React, { useId } from 'react';
import { Modal, ModalActions } from './Modal';
import { Button } from './Button';
import { useTranslation } from '@pierre/i18n';
import { ALERT_TONE } from './alertTone';

export type ConfirmDialogVariant = 'danger' | 'warning' | 'info';

export interface ConfirmDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirm: () => void;
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  variant?: ConfirmDialogVariant;
  isLoading?: boolean;
  icon?: React.ReactNode;
}

// The icon sits on its tone's tint in that tone's container ink (ALERT_TONE):
// a full `bg-error` disc stroked in `text-error` drew the glyph in the same
// colour as its disc, an empty circle in both themes.
const variantStyles: Record<ConfirmDialogVariant, { iconBg: string; iconColor: string; buttonVariant: 'danger' | 'primary' | 'secondary' }> = {
  danger: {
    iconBg: ALERT_TONE.error.tone,
    iconColor: ALERT_TONE.error.ink,
    buttonVariant: 'danger',
  },
  warning: {
    iconBg: ALERT_TONE.warning.tone,
    iconColor: ALERT_TONE.warning.ink,
    buttonVariant: 'primary',
  },
  info: {
    iconBg: ALERT_TONE.info.tone,
    iconColor: ALERT_TONE.info.ink,
    buttonVariant: 'primary',
  },
};

const defaultIcons: Record<ConfirmDialogVariant, React.ReactNode> = {
  danger: (
    <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
    </svg>
  ),
  warning: (
    <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
    </svg>
  ),
  info: (
    <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
    </svg>
  ),
};

export const ConfirmDialog: React.FC<ConfirmDialogProps> = ({
  isOpen,
  onClose,
  onConfirm,
  title,
  message,
  confirmLabel,
  cancelLabel,
  variant = 'danger',
  isLoading = false,
  icon,
}) => {
  const { t } = useTranslation();
  // Resolved in the body, not in the parameter list: a destructuring default
  // cannot call a hook, and both of these are copy rather than configuration.
  const confirmText = confirmLabel ?? t('common.confirm');
  const cancelText = cancelLabel ?? t('common.cancel');
  // The dialog is named by its own heading and described by its message:
  // without them a screen reader announced an unnamed dialog.
  const headingId = useId();
  const messageId = useId();
  const styles = variantStyles[variant];
  const displayIcon = icon || defaultIcons[variant];

  const handleConfirm = () => {
    onConfirm();
  };

  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      size="sm"
      showCloseButton={false}
      closeOnOverlayClick={!isLoading}
      closeOnEscape={!isLoading}
      labelledBy={headingId}
      describedBy={messageId}
    >
      <div className="text-center sm:text-left">
        <div className="mx-auto sm:mx-0 flex items-center justify-center sm:justify-start gap-4">
          <div className={`flex-shrink-0 w-12 h-12 rounded-full ${styles.iconBg} ${styles.iconColor} flex items-center justify-center`}>
            {displayIcon}
          </div>
          <div className="flex-1 min-w-0">
            <h3 id={headingId} className="text-lg font-semibold text-on-surface">
              {title}
            </h3>
          </div>
        </div>
        <p id={messageId} className="mt-3 text-sm text-on-surface sm:pl-16">
          {message}
        </p>
      </div>
      <ModalActions className="mt-6">
        <Button
          variant="secondary"
          onClick={onClose}
          disabled={isLoading}
        >
          {cancelText}
        </Button>
        <Button
          variant={styles.buttonVariant}
          onClick={handleConfirm}
          loading={isLoading}
          disabled={isLoading}
        >
          {confirmText}
        </Button>
      </ModalActions>
    </Modal>
  );
};
