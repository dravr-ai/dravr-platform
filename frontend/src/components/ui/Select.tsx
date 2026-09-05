// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Boreal Editorial Select — bottom-stroke underline, DESIGN.md §5
// ABOUTME: Same label, help and error treatment as Input; custom chevron affordance

import React, { forwardRef, useId } from 'react';

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface SelectProps extends Omit<React.SelectHTMLAttributes<HTMLSelectElement>, 'size'> {
  label?: string;
  error?: string;
  helpText?: string;
  options: SelectOption[];
  placeholder?: string;
  size?: 'sm' | 'md' | 'lg';
}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(
  ({ label, error, helpText, options, placeholder, size = 'md', className = '', id, ...props }, ref) => {
    const reactId = useId();
    const selectId = id || reactId;

    const sizeClasses = {
      sm: 'py-1.5 text-sm',
      md: 'py-2 text-sm',
      lg: 'py-3 text-base',
    };

    // The underline chrome lives in .boreal-underline-input — see Textarea for
    // why it cannot be an inline style.
    const baseClasses =
      'w-full bg-transparent text-on-surface font-sans appearance-none cursor-pointer ' +
      'focus:outline-none transition-colors duration-base disabled:cursor-not-allowed disabled:opacity-50';

    return (
      <div className="w-full">
        {label && (
          <label
            htmlFor={selectId}
            className="block text-sm font-medium text-on-surface-variant mb-2"
          >
            {label}
          </label>
        )}
        <div className="relative">
          <select
            ref={ref}
            id={selectId}
            aria-invalid={error ? true : undefined}
            className={`${baseClasses} ${sizeClasses[size]} pr-8 ${className} boreal-underline-input${
              error ? ' boreal-underline-input--error' : ''
            }`}
            {...props}
          >
            {placeholder && (
              <option value="" disabled>
                {placeholder}
              </option>
            )}
            {options.map((option) => (
              <option key={option.value} value={option.value} disabled={option.disabled}>
                {option.label}
              </option>
            ))}
          </select>
          <div className="absolute inset-y-0 right-0 flex items-center pointer-events-none text-on-surface-variant">
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
            </svg>
          </div>
        </div>
        {error && <p className="mt-1.5 text-xs text-error">{error}</p>}
        {helpText && !error && (
          <p className="mt-1.5 text-xs text-outline">{helpText}</p>
        )}
      </div>
    );
  }
);

Select.displayName = 'Select';
