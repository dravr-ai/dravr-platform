// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Boreal Editorial Input — bottom-stroke underline, DESIGN.md §5
// ABOUTME: Supports label, error/help text, left/right icons

import React, { forwardRef, useId } from 'react';

export interface InputProps extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'size'> {
  label?: string;
  error?: string;
  helpText?: string;
  leftIcon?: React.ReactNode;
  rightIcon?: React.ReactNode;
  size?: 'sm' | 'md' | 'lg';
  /**
   * Variant is retained for API stability but the Boreal system only ships
   * one editorial underline style. All three values render identically.
   */
  variant?: 'light' | 'dark' | 'glass';
}

export const Input = forwardRef<HTMLInputElement, InputProps>(
  ({ label, error, helpText, leftIcon, rightIcon, size = 'md', className = '', id, ...props }, ref) => {
    const reactId = useId();
    const inputId = id || reactId;

    const sizeClasses = {
      sm: 'py-1.5 text-sm',
      md: 'py-2 text-sm',
      lg: 'py-3 text-base',
    };

    const baseInputClasses =
      'w-full bg-transparent text-on-surface placeholder:text-outline font-sans ' +
      'focus:outline-none transition-colors duration-base disabled:cursor-not-allowed disabled:opacity-50';

    // The underline chrome lives in .boreal-underline-input, whose !important
    // declarations are what neutralize @tailwindcss/forms' full-border reset.
    // Only the icon gutters are left to inline style — an inline border here
    // would lose to those !important rules and never paint.
    const inputStyle: React.CSSProperties = {
      paddingLeft: leftIcon ? '1.5rem' : 0,
      paddingRight: rightIcon ? '1.5rem' : 0,
    };

    return (
      <div className="w-full">
        {label && (
          <label
            htmlFor={inputId}
            className="block text-[11px] font-medium font-label uppercase text-on-surface-variant mb-2"
            style={{ letterSpacing: '0.08em' }}
          >
            {label}
          </label>
        )}
        <div className="relative">
          {leftIcon && (
            <div className="absolute inset-y-0 left-0 flex items-center text-outline pointer-events-none">
              {leftIcon}
            </div>
          )}
          <input
            ref={ref}
            id={inputId}
            aria-invalid={error ? true : undefined}
            className={`${baseInputClasses} ${sizeClasses[size]} ${className} boreal-underline-input${
              error ? ' boreal-underline-input--error' : ''
            }`}
            style={inputStyle}
            {...props}
          />
          {rightIcon && (
            <div className="absolute inset-y-0 right-0 flex items-center text-outline">
              {rightIcon}
            </div>
          )}
        </div>
        {error && <p className="mt-1.5 text-xs text-error font-label">{error}</p>}
        {helpText && !error && (
          <p className="mt-1.5 text-xs text-outline font-label">{helpText}</p>
        )}
      </div>
    );
  }
);

Input.displayName = 'Input';
