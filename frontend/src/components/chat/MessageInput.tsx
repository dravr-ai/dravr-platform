// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat message input component with textarea, a "/" commands button and a send button
// ABOUTME: Handles keyboard shortcuts and the slash-command and @handle palettes

import { useRef, useEffect, useState, useCallback } from 'react';
import { clsx } from 'clsx';
import { Slash } from 'lucide-react';
import CommandPalette from '../CommandPalette';
import MentionPalette from './MentionPalette';
import { IconButton } from '../ui';
import { useCommandPalette } from '../../hooks/useCommandPalette';
import { useMentionPalette } from '../../hooks/useMentionPalette';
import { useTranslation } from '@pierre/i18n';

interface MessageInputProps {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  isStreaming: boolean;
  /** Whether input should be disabled (e.g., quota exceeded) */
  disabled?: boolean;
  /**
   * The open conversation, passed to the slash-command palette so group-scoped
   * commands are resolved for the group this conversation is bound to.
   */
  conversationId?: string | null;
}

export default function MessageInput({
  value,
  onChange,
  onSend,
  isStreaming,
  disabled = false,
  conversationId,
}: MessageInputProps) {
  const { t } = useTranslation();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const palette = useCommandPalette({ value, conversationId, onChange });

  // Where the caret sits, so a `@` typed mid-sentence still opens the mention
  // palette. Read off the textarea on every edit, click and keystroke; the
  // slash palette needs no caret because a command only ever opens the text.
  const [caret, setCaret] = useState(0);
  const syncCaret = useCallback(() => {
    setCaret(inputRef.current?.selectionStart ?? 0);
  }, []);
  // Inserting a mention rewrites the text; the caret has to land after the
  // inserted handle once React has committed the new value, not before.
  const pendingCaret = useRef<number | null>(null);
  const applyMention = useCallback(
    (next: string, nextCaret: number) => {
      pendingCaret.current = nextCaret;
      onChange(next);
    },
    [onChange],
  );
  useEffect(() => {
    const target = pendingCaret.current;
    if (target === null) return;
    pendingCaret.current = null;
    inputRef.current?.setSelectionRange(target, target);
    setCaret(target);
  }, [value]);
  const mentions = useMentionPalette({ value, caret, onChange: applyMention });

  // Focus input on mount
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // A palette owns Enter, Tab, the arrows and Escape while it is open:
    // Enter on a half-typed command or handle completes it rather than
    // sending it.
    if (palette.handleKeyDown(e)) return;
    if (mentions.handleKeyDown(e)) return;
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
    <div className="border-t ghost-border bg-surface px-4 pt-3 pb-[max(0.875rem,env(safe-area-inset-bottom))] md:px-8">
      <div className="max-w-3xl mx-auto">
        <CommandPalette
          matches={palette.matches}
          highlightedIndex={palette.highlightedIndex}
          onSelect={palette.select}
        />
        <MentionPalette
          matches={mentions.matches}
          highlightedIndex={mentions.highlightedIndex}
          onSelect={mentions.select}
        />
        <div className="flex items-end gap-2">
          {/* The visible affordance that `/` exists at all — Telegram's bot
              "Menu" button, in the one place a new athlete is already
              looking. It types the character the palette watches for rather
              than opening a second, parallel list. */}
          <IconButton
            variant="ghost"
            aria-label={t('chat.commandsLabel')}
            title={t('chat.commandsLabel')}
            data-testid="slash-command-button"
            disabled={isStreaming || disabled}
            onClick={() => {
              onChange('/');
              inputRef.current?.focus();
            }}
          >
            <Slash className="w-4 h-4" aria-hidden="true" />
          </IconButton>
          <div className="relative flex-1 min-w-0">
          {/* The composer is a chat surface, not a form field — DESIGN.md §5
              lists the two separately. It keeps its enclosing rounded field so
              the embedded 44x44 send button has something to sit inside; the
              editorial underline has no box to host it. The field is the one
              filled shape on the canvas, so it carries no hairline of its own. */}
          {/* eslint-disable-next-line no-restricted-syntax */}
          <textarea
            ref={inputRef}
            value={value}
            onChange={(e) => {
              onChange(e.target.value);
              setCaret(e.target.selectionStart ?? e.target.value.length);
            }}
            onKeyDown={handleKeyDown}
            onKeyUp={syncCaret}
            onClick={syncCaret}
            onSelect={syncCaret}
            placeholder={t('chat.messageDravrPlaceholder')}
            className="w-full resize-none rounded-xl border-0 bg-surface-container-low text-on-surface placeholder:text-outline pl-4 pr-16 py-3 focus:outline-none focus:ring-2 focus:ring-primary/40 text-sm transition-colors overflow-hidden"
            rows={1}
            disabled={isStreaming || disabled}
          />
          <button
            onClick={onSend}
            disabled={!value.trim() || isStreaming || disabled}
            aria-label={t('chat.sendMessageAria')}
            className={clsx(
              'absolute right-2 top-1/2 -translate-y-1/2 min-w-[44px] min-h-[44px] flex items-center justify-center rounded-full transition-colors',
              value.trim() && !isStreaming && !disabled
                ? 'bg-primary text-on-primary hover:bg-primary-hover'
                : 'text-on-surface-variant cursor-not-allowed'
            )}
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
            </svg>
          </button>
          </div>
        </div>
        <div className="flex items-center justify-center gap-2 mt-2">
          <p className="text-xs text-outline hidden sm:block">
            {t('chat.inputKeyHint')}
          </p>
        </div>
      </div>
    </div>
  );
}
