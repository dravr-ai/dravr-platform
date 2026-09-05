// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Boreal Checkbox and Radio — the choice controls DESIGN.md §5 lacked
// ABOUTME: Label + description sit beside the control, sharing Input's help/error voice

import React, { forwardRef, useId } from 'react';

// Input, Textarea and Select cover text entry; until these existed, every
// checkbox and radio in the app was hand-rolled, which is the same hole that
// let a boxed textarea sit beside an editorial underline for months.
const CONTROL =
  'w-4 h-4 shrink-0 ghost-border bg-surface-container-low text-primary ' +
  'focus:ring-2 focus:ring-primary focus:ring-offset-0 disabled:opacity-50 disabled:cursor-not-allowed';

interface ChoiceProps {
  label: string;
  /** Secondary line under the label, in the same voice as Input's helpText. */
  description?: string;
  error?: string;
}

export interface CheckboxProps
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'type' | 'size'>,
    ChoiceProps {}

export const Checkbox = forwardRef<HTMLInputElement, CheckboxProps>(
  ({ label, description, error, className = '', id, disabled, ...props }, ref) => {
    const reactId = useId();
    const inputId = id || reactId;

    return (
      <div className="w-full">
        <label
          htmlFor={inputId}
          className={`flex items-start gap-3 ${disabled ? 'cursor-not-allowed' : 'cursor-pointer'}`}
        >
          <input
            ref={ref}
            id={inputId}
            type="checkbox"
            disabled={disabled}
            aria-invalid={error ? true : undefined}
            className={`${CONTROL} rounded mt-0.5 ${className}`}
            {...props}
          />
          <span className="min-w-0">
            <span className="block text-sm text-on-surface">{label}</span>
            {description && (
              <span className="block text-xs text-outline mt-0.5">{description}</span>
            )}
          </span>
        </label>
        {error && <p className="mt-1.5 text-xs text-error">{error}</p>}
      </div>
    );
  }
);

Checkbox.displayName = 'Checkbox';

export interface RadioProps
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'type' | 'size'>,
    ChoiceProps {}

export const Radio = forwardRef<HTMLInputElement, RadioProps>(
  ({ label, description, error, className = '', id, disabled, ...props }, ref) => {
    const reactId = useId();
    const inputId = id || reactId;

    return (
      <div className="w-full">
        <label
          htmlFor={inputId}
          className={`flex items-start gap-3 ${disabled ? 'cursor-not-allowed' : 'cursor-pointer'}`}
        >
          <input
            ref={ref}
            id={inputId}
            type="radio"
            disabled={disabled}
            aria-invalid={error ? true : undefined}
            className={`${CONTROL} rounded-full mt-0.5 ${className}`}
            {...props}
          />
          <span className="min-w-0">
            <span className="block text-sm text-on-surface">{label}</span>
            {description && (
              <span className="block text-xs text-outline mt-0.5">{description}</span>
            )}
          </span>
        </label>
        {error && <p className="mt-1.5 text-xs text-error">{error}</p>}
      </div>
    );
  }
);

Radio.displayName = 'Radio';
