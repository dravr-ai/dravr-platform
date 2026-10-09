// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A button that opens the system file picker and hands back the one file chosen — the file-input primitive
// ABOUTME: The native input stays hidden; picking the same file twice still reports it, and a cancelled pick reports nothing

import { useRef, type ChangeEvent, type ReactNode } from 'react';

export interface FilePickerButtonProps {
  /** The file types the picker offers, as the input's `accept` attribute takes them. */
  accept: string;
  /** Called with the file the person chose. */
  onPick: (file: File) => void;
  /** The button's visible text. */
  children: ReactNode;
  disabled?: boolean;
  /** The button's accessible name, when its text alone does not say enough. */
  'aria-label'?: string;
  className?: string;
  /** Test id of the button; the hidden input carries it with `-input` appended. */
  'data-testid'?: string;
}

export function FilePickerButton({
  accept,
  onPick,
  children,
  disabled = false,
  className,
  'aria-label': ariaLabel,
  'data-testid': testId,
}: FilePickerButtonProps) {
  const input = useRef<HTMLInputElement>(null);
  const onChange = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    // Cleared, so picking the same file again still fires a change.
    event.target.value = '';
    if (file !== undefined) onPick(file);
  };
  return (
    <>
      <input
        ref={input}
        type="file"
        accept={accept}
        className="hidden"
        tabIndex={-1}
        aria-hidden="true"
        data-testid={testId === undefined ? undefined : `${testId}-input`}
        onChange={onChange}
      />
      <button
        type="button"
        aria-label={ariaLabel}
        disabled={disabled}
        onClick={() => input.current?.click()}
        className={className}
        data-testid={testId}
      >
        {children}
      </button>
    </>
  );
}
