// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat message input component with textarea and send button
// ABOUTME: Handles keyboard shortcuts and ideas popover

import { useRef, useEffect } from 'react';
import { clsx } from 'clsx';
import PromptSuggestions from '../PromptSuggestions';

interface MessageInputProps {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  isStreaming: boolean;
  /** Whether input should be disabled (e.g., quota exceeded) */
  disabled?: boolean;
  showIdeas: boolean;
  onToggleIdeas: () => void;
  onSelectPrompt: (prompt: string, coachId?: string) => void;
}

export default function MessageInput({
  value,
  onChange,
  onSend,
  isStreaming,
  disabled = false,
  showIdeas,
  onToggleIdeas,
  onSelectPrompt,
}: MessageInputProps) {
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Focus input on mount
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // On touch (coarse-pointer) soft keyboards the Return key must insert a
    // newline — there is a dedicated 44x44 Send button. Enter-to-send is kept
    // on pointer-fine (desktop/laptop) devices only.
    const coarse = typeof window !== 'undefined' && window.matchMedia('(pointer: coarse)').matches;
    if (e.key === 'Enter' && !e.shiftKey && !coarse) {
      e.preventDefault();
      onSend();
    }
  };

  return (
    <div className="border-t ghost-border p-4 pb-[max(1rem,env(safe-area-inset-bottom))] bg-surface-container-low">
      <div className="max-w-3xl mx-auto">
        {/* Ideas popover */}
        {showIdeas && (
          <div className="mb-4 p-4 bg-surface-container-low rounded-xl border ghost-border relative">
            <button
              onClick={onToggleIdeas}
              className="absolute top-2 right-2 text-outline hover:text-on-surface transition-colors"
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
            <p className="text-xs text-on-surface-variant mb-3">Click a suggestion to fill the input:</p>
            <PromptSuggestions onSelectPrompt={onSelectPrompt} />
          </div>
        )}
        <div className="relative">
          {/* The composer is a chat surface, not a form field — DESIGN.md §5
              lists the two separately. It keeps its enclosing rounded field so
              the embedded 44x44 send button has something to sit inside; the
              editorial underline has no box to host it. */}
          {/* eslint-disable-next-line no-restricted-syntax */}
          <textarea
            ref={inputRef}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Message Dravr..."
            className="w-full resize-none rounded-xl border ghost-border bg-surface-container-low text-on-surface placeholder:text-outline pl-4 pr-16 py-3 focus:outline-none focus:ring-2 focus:ring-primary/30 focus:border-primary text-sm transition-colors overflow-hidden"
            rows={1}
            disabled={isStreaming || disabled}
          />
          <button
            onClick={onSend}
            disabled={!value.trim() || isStreaming || disabled}
            aria-label="Send message"
            className={clsx(
              'absolute right-2 top-1/2 -translate-y-1/2 min-w-[44px] min-h-[44px] flex items-center justify-center rounded-lg transition-colors',
              value.trim() && !isStreaming && !disabled
                ? 'bg-primary text-on-primary hover:bg-primary/90 shadow-ambient'
                : 'text-on-surface-variant cursor-not-allowed'
            )}
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
            </svg>
          </button>
        </div>
        <div className="flex items-center justify-center gap-2 mt-2">
          <p className="text-xs text-outline hidden sm:block">
            Press Enter to send, Shift+Enter for new line
          </p>
          <span className="text-on-surface-variant hidden sm:inline">|</span>
          <button
            onClick={onToggleIdeas}
            className="text-xs text-primary hover:text-primary-fixed-dim flex items-center gap-1 transition-colors"
          >
            <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
            </svg>
            Need ideas?
          </button>
        </div>
      </div>
    </div>
  );
}
