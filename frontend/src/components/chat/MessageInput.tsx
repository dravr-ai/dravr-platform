// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat message input component with textarea, a "/" commands button and a send button
// ABOUTME: Handles keyboard shortcuts, the slash-command and @handle palettes, and a coach's Everyone / Only <agent> switch

import { useRef, useEffect, useLayoutEffect, useState, useCallback, useId } from 'react';
import { clsx } from 'clsx';
import { Lock, Quote, Slash, Square, Users, X } from 'lucide-react';
import type { MessageAudience, RoomQuote } from '@pierre/shared-types';
import { PRIVATE_LANE_KEYS, privateLaneComposerText } from '@pierre/chat-utils';
import CommandPalette from '../CommandPalette';
import MentionPalette from './MentionPalette';
import { IconButton } from '../ui';
import { useCommandPalette } from '../../hooks/useCommandPalette';
import { useMentionPalette } from '../../hooks/useMentionPalette';
import { useTranslation } from '@pierre/i18n';

/**
 * The most lines the composer shows before it scrolls, so a long draft never
 * pushes the thread off the screen.
 */
const COMPOSER_MAX_LINES = 8;

/**
 * The box each composer icon sits in: pinned to the field's bottom edge and as
 * tall as the field holding one line (`py-2`, the 23px line and two 1px
 * borders; `py-3` on a coarse pointer), so an icon shares the last line's
 * centre whatever height its button takes.
 */
const COMPOSER_ICON_SLOT =
  'absolute bottom-0 z-10 flex h-[41px] items-center [@media(pointer:coarse)]:h-[49px]';

/**
 * Where the text starts after a measured left slot: the slot's own 6px inset
 * from the field's edge, then a 4px gap before the first character.
 */
const COMPOSER_TEXT_GAP = 10;

/**
 * A coach's private lane in a room of one athlete (carnet#1249), as the
 * composer draws it: the Everyone / Only <agent> switch, and the athlete's
 * message "Adjust plan from this" quoted.
 */
export interface ComposerPrivateLane {
  /** Who the next send reaches. */
  audience: MessageAudience;
  /** The room's agent, as the room names it. */
  agentName: string;
  /** The room's one athlete. */
  athleteName: string;
  onToggle: () => void;
  /** The athlete's room message held for the next private send. */
  quote: RoomQuote | null;
  onRemoveQuote: () => void;
}

interface MessageInputProps {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  isStreaming: boolean;
  /**
   * Ends the running turn. While a turn streams, the send button is this
   * stop button instead — the athlete's one way to end a reply on purpose.
   */
  onStop: () => void;
  /** A stop was sent and the turn has not ended yet; the button waits. */
  isStopping?: boolean;
  /** Whether input should be disabled (e.g., quota exceeded) */
  disabled?: boolean;
  /**
   * The open conversation, passed to the slash-command palette so group-scoped
   * commands are resolved for the group this conversation is bound to.
   */
  conversationId?: string | null;
  /**
   * Whether the composer takes focus when it mounts. The chat surface's does;
   * one embedded under an activity does not, since focusing it would scroll
   * the page past what the athlete opened and raise a phone's keyboard.
   */
  focusOnMount?: boolean;
  /**
   * A coach's private lane in this room. Present, the composer carries the
   * Everyone / Only <agent> switch; while private the field is drawn dashed
   * in the private accent and says the athlete won't see what is typed.
   */
  privateLane?: ComposerPrivateLane | null;
}

export default function MessageInput({
  value,
  onChange,
  onSend,
  isStreaming,
  onStop,
  isStopping = false,
  disabled = false,
  conversationId,
  focusOnMount = true,
  privateLane = null,
}: MessageInputProps) {
  const { t } = useTranslation();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const isPrivate = privateLane?.audience === 'private';
  // What the switch and the field say for the audience the next send reaches.
  const laneText = privateLane
    ? privateLaneComposerText(privateLane.audience, { agent: privateLane.agentName, athlete: privateLane.athleteName }, t)
    : null;
  const placeholder = laneText?.placeholder ?? t('chat.messageDravrPlaceholder');
  // With the switch, the field's name says who reads it; the placeholder
  // already does, and it is gone once the coach types.
  const fieldLabel = laneText?.placeholder ?? t('chat.messageDravrLabel');

  // The switch sits in the field beside the slash, and its label is as wide
  // as the language spells it: the text starts after whatever the left slot
  // holds, measured rather than guessed.
  const leftSlotRef = useRef<HTMLDivElement>(null);
  const [leftInset, setLeftInset] = useState<number | null>(null);
  const hasLane = privateLane !== null;
  useLayoutEffect(() => {
    const slot = leftSlotRef.current;
    if (!hasLane || !slot) {
      setLeftInset(null);
      return;
    }
    const measure = () => setLeftInset(slot.offsetWidth > 0 ? slot.offsetWidth : null);
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(slot);
    return () => observer.disconnect();
  }, [hasLane]);

  // "Adjust plan from this" hands the coach to the composer: a new quote
  // puts the caret in the field.
  const quoteId = privateLane?.quote?.entry_id ?? null;
  useEffect(() => {
    if (quoteId !== null) inputRef.current?.focus();
  }, [quoteId]);
  // Unique per composer: the chat surface and an activity's thread can both
  // be mounted, and a duplicated id would point a label at the wrong field.
  const inputId = useId();
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

  // The field grows with what is typed, one line at a time, up to the cap and
  // then scrolls — the way every messenger composer behaves. Measured after
  // React commits the value so a paste or a cleared send resizes at once. The
  // cap is read off the field's own line height and padding, which differ
  // between a fine and a coarse pointer, so it is eight whole lines on both.
  // An empty field is as tall as its placeholder, which changes with the
  // audience a coach's switch picks, as the left padding does.
  useLayoutEffect(() => {
    const field = inputRef.current;
    if (!field) return;
    field.style.height = 'auto';
    const border = field.offsetHeight - field.clientHeight;
    const wanted = field.scrollHeight + border;
    const style = getComputedStyle(field);
    const cap =
      COMPOSER_MAX_LINES * parseFloat(style.lineHeight) +
      parseFloat(style.paddingTop) +
      parseFloat(style.paddingBottom) +
      border;
    field.style.height = `${Math.min(wanted, cap)}px`;
    field.style.overflowY = wanted > cap ? 'auto' : 'hidden';
  }, [value, leftInset, placeholder]);

  // Focus input on mount
  useEffect(() => {
    if (focusOnMount) inputRef.current?.focus();
  }, [focusOnMount]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // A palette owns Enter, Tab, the arrows and Escape while it is open:
    // Enter on a half-typed command or handle completes it rather than
    // sending it.
    if (palette.handleKey(e.key) || mentions.handleKey(e.key)) {
      e.preventDefault();
      return;
    }
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
    <div className="bg-surface px-4 pt-2 pb-[max(1rem,env(safe-area-inset-bottom))] md:px-6">
      <div className="mx-auto max-w-[720px]">
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
        {/* The athlete's message the next private send quotes, until it is
            sent or the coach takes it off. */}
        {privateLane?.quote ? (
          <div
            data-testid="composer-quote"
            className="mb-2 flex min-w-0 items-center gap-2 rounded-lg border border-dashed border-warning/60 bg-warning/5 py-0.5 pl-2.5 pr-0.5"
          >
            <Quote className="h-3.5 w-3.5 shrink-0 text-on-warning-container" aria-hidden="true" />
            <span className="shrink-0 text-xs font-medium text-on-warning-container">
              {t(PRIVATE_LANE_KEYS.quoteChip, { name: privateLane.quote.author_display_name })}
            </span>
            <span className="min-w-0 flex-1 truncate text-xs text-on-surface-variant">{privateLane.quote.content}</span>
            <IconButton
              variant="ghost"
              size="sm"
              aria-label={t(PRIVATE_LANE_KEYS.quoteRemove)}
              title={t(PRIVATE_LANE_KEYS.quoteRemove)}
              data-testid="composer-quote-remove"
              onClick={() => {
                privateLane.onRemoveQuote();
                inputRef.current?.focus();
              }}
            >
              <X className="h-3.5 w-3.5" aria-hidden="true" />
            </IconButton>
          </div>
        ) : null}
        {/* The composer is a chat surface, not a form field — DESIGN.md §5
            lists the two separately. It is the one white shape on the paper:
            a 41px field with a hairline, radius 12, the slash on its left and
            the send on its right, both inside it. The measured chat products
            all draw the composer lighter than the page, never darker, and
            none of them puts a helper line under it — the placeholder says
            what `/` does. */}
        <div className="relative min-w-0">
          {/* The visible affordance that `/` exists at all — Telegram's bot
              "Menu" button, in the one place a new athlete is already
              looking. It types the character the palette watches for rather
              than opening a second, parallel list. */}
          {/* Each icon is centred on the field's last line by a box one
              line tall, never lifted off the bottom by a fixed amount:
              `touch-target` grows a button to 44px below 1024px wide and on
              a coarse pointer, and a fixed offset then puts its icon above
              the text. */}
          <div ref={leftSlotRef} className={clsx(COMPOSER_ICON_SLOT, 'left-1.5 gap-1')}>
            {/* Who the next send reaches — Everyone, or the agent alone. A
                private message sent to the room by mistake is the costlier
                error, so the switch is explicit and says what a press does.
                Its name starts with the label it shows and carries the state
                (WCAG 2.5.3), so it announces no pressed state on top. */}
            {privateLane && laneText ? (
              <button
                type="button"
                onClick={privateLane.onToggle}
                aria-label={laneText.aria}
                title={laneText.aria}
                data-testid="composer-audience-switch"
                className={clsx(
                  'touch-target inline-flex h-7 shrink-0 items-center justify-center gap-1 whitespace-nowrap rounded-lg px-2 text-xs font-semibold transition-colors focus-ring',
                  isPrivate
                    ? 'bg-warning/15 text-on-warning-container hover:bg-warning/25'
                    : 'bg-surface-container-low text-on-surface-variant hover:bg-surface-container hover:text-on-surface',
                )}
              >
                {isPrivate ? (
                  <Lock className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                ) : (
                  <Users className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                )}
                {/* On a phone the icon carries it: the label would leave the field no room. */}
                <span className="hidden sm:inline">{laneText.label}</span>
              </button>
            ) : null}
            {/* The private placeholder offers no commands: the coach is asking
                the agent for a change, so the slash steps aside. */}
            {isPrivate ? null : (
              <IconButton
                variant="ghost"
                size="sm"
                aria-label={t('chat.commandsLabel')}
                title={t('chat.commandsLabel')}
                data-testid="slash-command-button"
                disabled={isStreaming || disabled}
                onClick={() => {
                  onChange('/');
                  inputRef.current?.focus();
                }}
              >
                <Slash className="h-4 w-4" aria-hidden="true" />
              </IconButton>
            )}
          </div>
          {/* eslint-disable-next-line no-restricted-syntax */}
          <textarea
            ref={inputRef}
            id={inputId}
            name="message"
            // Marks the field useComposerFocused watches: a phone folds its
            // tab bar and Home's Today line away while the athlete types.
            data-composer="true"
            // The placeholder disappears as soon as the athlete types, so it
            // cannot be the field's name; a screen reader announces this one.
            aria-label={fieldLabel}
            data-audience={privateLane ? privateLane.audience : undefined}
            value={value}
            onChange={(e) => {
              onChange(e.target.value);
              setCaret(e.target.selectionStart ?? e.target.value.length);
            }}
            onKeyDown={handleKeyDown}
            onKeyUp={syncCaret}
            onClick={syncCaret}
            onSelect={syncCaret}
            placeholder={placeholder}
            // While private the field is drawn dashed in the private accent,
            // and keeps that edge while focused: the cue matters most then.
            className={clsx(
              'block w-full resize-none overflow-hidden rounded-xl border py-2 pl-11 pr-12 text-base text-on-surface transition-colors placeholder:text-outline focus:outline-none focus:ring-2 [@media(pointer:coarse)]:min-h-[48px] [@media(pointer:coarse)]:py-3',
              isPrivate
                ? 'border-dashed border-warning/60 bg-warning/5 focus:ring-warning/30'
                : 'ghost-border bg-surface-container-lowest focus:border-transparent focus:ring-primary/40',
            )}
            // The measured slot: the left padding clears the switch and the slash.
            style={leftInset !== null ? { paddingLeft: leftInset + COMPOSER_TEXT_GAP } : undefined}
            rows={1}
            disabled={isStreaming || disabled}
          />
          <div className={clsx(COMPOSER_ICON_SLOT, 'right-1.5')}>
            {isStreaming ? (
              <button
                type="button"
                onClick={onStop}
                disabled={isStopping}
                aria-label={t('chat.stopTurnAria')}
                title={t('chat.stopTurnAria')}
                data-testid="stop-turn-button"
                className={clsx(
                  'flex h-7 w-7 items-center justify-center rounded-full transition-colors touch-target',
                  isStopping
                    ? 'text-on-surface-variant cursor-not-allowed'
                    : 'bg-primary text-on-primary hover:bg-primary-hover'
                )}
              >
                <Square className="h-3 w-3 fill-current" aria-hidden="true" />
              </button>
            ) : (
              <button
                type="button"
                onClick={onSend}
                disabled={!value.trim() || disabled}
                aria-label={t('chat.sendMessageAria')}
                className={clsx(
                  'flex h-7 w-7 items-center justify-center rounded-full transition-colors touch-target',
                  value.trim() && !disabled
                    ? 'bg-primary text-on-primary hover:bg-primary-hover'
                    : 'text-on-surface-variant cursor-not-allowed'
                )}
              >
                <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
                </svg>
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
