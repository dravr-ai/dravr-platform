// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Boreal Editorial Textarea — bottom-stroke underline, DESIGN.md §5
// ABOUTME: Multi-line sibling of Input; same label, help and error treatment

import React, { forwardRef, useId } from 'react';

export interface TextareaProps extends Omit<React.TextareaHTMLAttributes<HTMLTextAreaElement>, 'size'> {
  label?: string;
  error?: string;
  helpText?: string;
  size?: 'sm' | 'md' | 'lg';
}

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ label, error, helpText, size = 'md', className = '', id, rows = 3, ...props }, ref) => {
    const reactId = useId();
    const textareaId = id || reactId;

    const sizeClasses = {
      sm: 'py-1.5 text-sm',
      md: 'py-2 text-sm',
      lg: 'py-3 text-base',
    };

    // The underline chrome (border, radius, background, focus growth) lives in
    // .boreal-underline-input so it can carry the !important needed to beat
    // @tailwindcss/forms' full-border reset.
    const baseClasses =
      'w-full bg-transparent text-on-surface placeholder:text-outline font-sans resize-none ' +
      'focus:outline-none transition-colors duration-base disabled:cursor-not-allowed disabled:opacity-50';

    return (
      <div className="w-full">
        {label && (
          <label
            htmlFor={textareaId}
            className="block text-sm font-medium text-on-surface-variant mb-2"
          >
            {label}
          </label>
        )}
        <textarea
          ref={ref}
          id={textareaId}
          rows={rows}
          aria-invalid={error ? true : undefined}
          className={`${baseClasses} ${sizeClasses[size]} ${className} boreal-underline-input${
            error ? ' boreal-underline-input--error' : ''
          }`}
          {...props}
        />
        {error && <p className="mt-1.5 text-xs text-error">{error}</p>}
        {helpText && !error && (
          <p className="mt-1.5 text-xs text-outline">{helpText}</p>
        )}
      </div>
    );
  }
);

Textarea.displayName = 'Textarea';
