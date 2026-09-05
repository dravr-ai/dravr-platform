// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reusable Modal component with Pierre design system styling
// ABOUTME: Features smooth animations, gradient accent bar, and accessible focus management

import React, { useId } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from '@pierre/i18n';
import { useDialog } from '../../hooks/useDialog';

export interface ModalProps {
  isOpen: boolean;
  onClose: () => void;
  title?: string;
  children: React.ReactNode;
  size?: 'sm' | 'md' | 'lg' | 'xl' | '2xl' | '3xl';
  showCloseButton?: boolean;
  closeOnOverlayClick?: boolean;
  closeOnEscape?: boolean;
  footer?: React.ReactNode;
}

export const Modal: React.FC<ModalProps> = ({
  isOpen,
  onClose,
  title,
  children,
  size = 'md',
  showCloseButton = true,
  closeOnOverlayClick = true,
  closeOnEscape = true,
  footer,
}) => {
  const { t } = useTranslation();
  // Focus trap, focus restore, Escape and the refcounted scroll lock all live
  // in useDialog, shared with the two provider modals that render their own
  // overlay rather than composing this one.
  const { containerRef } = useDialog({ open: isOpen, onClose, closeOnEscape });
  // A unique id per instance. This was the literal string 'modal-title', so two
  // mounted dialogs put duplicate ids in the document and aria-labelledby
  // resolved to whichever came first.
  const titleId = useId();

  const sizeClasses = {
    sm: 'max-w-sm',
    md: 'max-w-md',
    lg: 'max-w-lg',
    xl: 'max-w-xl',
    '2xl': 'max-w-2xl',
    '3xl': 'max-w-3xl',
  };

  const handleOverlayClick = (event: React.MouseEvent<HTMLDivElement>) => {
    if (closeOnOverlayClick && event.target === event.currentTarget) {
      onClose();
    }
  };

  if (!isOpen) return null;

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm animate-fade-in"
      onClick={handleOverlayClick}
      role="dialog"
      aria-modal="true"
      aria-labelledby={title ? titleId : undefined}
    >
      <div
        ref={containerRef}
        tabIndex={-1}
        className={`${sizeClasses[size]} w-full max-h-[90dvh] flex flex-col bg-surface-container-low border ghost-border rounded-xl overflow-hidden animate-scale-in`}
      >
        {/* Gradient accent bar */}
        <div className="h-1 w-full boreal-hero-gradient flex-shrink-0" />

        {/* Header */}
        {(title || showCloseButton) && (
          <div className="flex items-center justify-between px-6 py-4 border-b ghost-border flex-shrink-0">
            {title && (
              <h2 id={titleId} className="text-lg font-semibold text-on-surface">
                {title}
              </h2>
            )}
            {showCloseButton && (
              <button
                type="button"
                onClick={onClose}
                className="p-2 text-on-surface-variant hover:text-on-surface hover:bg-surface-container rounded-lg transition-colors touch-target flex items-center justify-center"
                aria-label={t('shell.modalCloseAria')}
              >
                <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            )}
          </div>
        )}

        {/* Content — scrolls within max-h so tall forms never push the footer
            (submit button) off a short/landscape phone viewport. min-h-0 lets
            this flex child actually shrink to enable the scroll. */}
        <div className="flex-1 min-h-0 overflow-y-auto px-6 py-4 text-on-surface">{children}</div>

        {/* Footer */}
        {footer && (
          <div className="px-6 py-4 bg-surface/50 border-t ghost-border flex-shrink-0">{footer}</div>
        )}
      </div>
    </div>,
    document.body
  );
};

// Convenience component for modal actions
export interface ModalActionsProps {
  children: React.ReactNode;
  className?: string;
}

export const ModalActions: React.FC<ModalActionsProps> = ({ children, className = '' }) => {
  return <div className={`flex items-center justify-end gap-3 ${className}`}>{children}</div>;
};
