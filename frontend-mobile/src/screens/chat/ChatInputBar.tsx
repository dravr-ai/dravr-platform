// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat composer bar — the field, the slash-command and @handle palettes, voice and send
// ABOUTME: An in-flow bar under the thread; in a coach's one-athlete room it carries the Everyone / Only <agent> switch and the quote

import React, { useEffect, useState } from 'react';
import { View, TextInput, TouchableOpacity, Pressable, ActivityIndicator, Text, StyleSheet, Keyboard, Platform } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { Ionicons } from '@expo/vector-icons';
import { isCommandDraft } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { PALETTE_KEYS } from '@pierre/ui-logic';
import type { PrivateLaneComposerText } from '@pierre/chat-utils';
import type { MessageAudience } from '@pierre/shared-types';
import { useThemeColors } from '../../constants/theme';
import { VoiceButton } from '../../components/ui';
import { CommandPalette } from '../../components/CommandPalette';
import { MentionPalette } from '../../components/MentionPalette';
import { useCommandPalette } from '../../hooks/useCommandPalette';
import { useMentionPalette } from '../../hooks/useMentionPalette';
import { composerKey, type ComposerKeyEvent } from '../../hooks/composerKeys';

/**
 * The Everyone / Only <agent> switch a coach sees in a room of one athlete
 * (carnet#1249). Who the next send reaches; the host routes it.
 */
export interface ComposerAudience {
  /** `private` while the next send goes to the agent alone. */
  value: MessageAudience;
  /** The switch's label and accessible name, and the field's placeholder, for `value`. */
  text: PrivateLaneComposerText;
  onToggle: () => void;
}

/** The athlete's room message "Adjust plan from this" quoted, until it is sent or removed. */
export interface ComposerQuote {
  /** "From <name>'s message". */
  label: string;
  /** The remove control's accessible name. */
  removeLabel: string;
  onRemove: () => void;
}

interface ChatInputBarProps {
  inputText: string;
  partialTranscript: string;
  isListening: boolean;
  /**
   * A turn is running in this thread — sent from here, or one the server is
   * still answering after the app reopened. The send slot is the stop button.
   */
  isSending: boolean;
  /** A stop was sent and the turn has not ended yet; the stop button waits. */
  isStopping: boolean;
  /** When true, input and send are disabled (e.g. usage quota blocked) */
  disabled?: boolean;
  voiceAvailable: boolean;
  inputRef: React.RefObject<TextInput | null>;
  onChangeText: (text: string) => void;
  onVoicePress: () => void;
  onSendMessage: () => void;
  /** Ends the running turn; while one runs, the send button is this instead. */
  onStopTurn: () => void;
  /**
   * The audience switch of a coach's one-athlete room. Absent for everyone
   * else and in every other thread, where the bar has no such control.
   */
  audience?: ComposerAudience;
  /** The quote the next private send carries, shown as a chip over the field. */
  quote?: ComposerQuote | null;
}

/**
 * The bar is paper with a top hairline, laid out under the message list; the
 * screen's `KeyboardAvoidingView` moves it with the keyboard. Its leading slot
 * is empty — typing `/` opens the command palette, and the empty thread's
 * hint sentence is what tells a new athlete so (Boreal v2.2 P3.3, P3.4).
 */
/** The bar's own breathing room under the field, keyboard up or down. */
const BAR_BOTTOM_PADDING = 8;

/**
 * The bar rests on the home indicator, so it pays the safe-area inset while
 * the keyboard is down; once the keyboard is up the `KeyboardAvoidingView`
 * has lifted the whole bar above it, and the inset would only be a dead band
 * between the field and the keys.
 */
export function composerBottomPadding(keyboardShown: boolean, bottomInset: number): number {
  return keyboardShown ? BAR_BOTTOM_PADDING : bottomInset + BAR_BOTTOM_PADDING;
}

export function ChatInputBar({
  inputText,
  partialTranscript,
  isListening,
  isSending,
  isStopping,
  disabled = false,
  voiceAvailable,
  inputRef,
  onChangeText,
  onVoicePress,
  onSendMessage,
  onStopTurn,
  audience,
  quote = null,
}: ChatInputBarProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const insets = useSafeAreaInsets();
  const [keyboardShown, setKeyboardShown] = useState(false);
  useEffect(() => {
    // iOS announces the keyboard before it moves; Android only after.
    const showEvent = Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow';
    const hideEvent = Platform.OS === 'ios' ? 'keyboardWillHide' : 'keyboardDidHide';
    const show = Keyboard.addListener(showEvent, () => setKeyboardShown(true));
    const hide = Keyboard.addListener(hideEvent, () => setKeyboardShown(false));
    return () => {
      show.remove();
      hide.remove();
    };
  }, []);
  const displayText = isListening ? partialTranscript : inputText;
  // Dictation is prose, never a command, so the palettes read the typed text
  // rather than what is on screen mid-transcription.
  const paletteValue = isListening ? '' : inputText;
  const palette = useCommandPalette({ value: paletteValue, onChange: onChangeText });
  // Where the athlete is typing. The mention grammar reads the token that
  // ends at the caret, so a `@` in the middle of a message opens the palette
  // where it was typed rather than at the end of the text. Until the input
  // reports a selection the caret is the end of the text, which is where
  // typing lands.
  const [selectionEnd, setSelectionEnd] = useState<number | null>(null);
  const caret = selectionEnd === null || selectionEnd > inputText.length ? inputText.length : selectionEnd;
  const handleSelectionChange = (event: { nativeEvent: { selection: { end: number } } }) => {
    setSelectionEnd(event.nativeEvent.selection.end);
  };
  // An "@" offers the athlete's installed coaches by handle; the pick is
  // inserted lowercase and verbatim, whatever case the keyboard typed.
  const mentions = useMentionPalette({
    value: paletteValue,
    caret,
    onChange: (value, nextCaret) => {
      onChangeText(value);
      setSelectionEnd(nextCaret);
    },
  });
  const canSend = Boolean(inputText.trim()) && !isSending && !isListening && !disabled;

  /**
   * Hardware-keyboard keys, offered to the command palette first and the
   * mention palette second — only one of the two is ever open. What neither
   * takes falls through to the field, except Enter on a finished command:
   * the athlete typed the whole thing and means to send it.
   */
  const handleKeyPress = (event: ComposerKeyEvent) => {
    const key = composerKey(event);
    if (palette.handleKey(key)) return;
    if (mentions.handleKey(key)) return;
    if (key === PALETTE_KEYS.enter && canSend && isCommandDraft(inputText)) {
      onSendMessage();
    }
  };

  // One slot, two jobs: send while the athlete is composing, stop while a
  // turn runs. A stop already sent waits, unfilled, for the stream to end.
  const slot = isSending
    ? {
        filled: !isStopping,
        disabled: isStopping,
        onPress: onStopTurn,
        label: t('chat.stopTurnAria'),
        testID: 'stop-turn-button',
      }
    : {
        filled: canSend,
        disabled: !canSend,
        onPress: onSendMessage,
        label: t('app.composerSendAria'),
        testID: canSend ? 'send-button' : 'send-button-disabled',
      };

  // While the next send reaches the agent alone, the field is drawn dashed in
  // the private accent, as the private bubbles are: a line meant for the
  // agent sent to the room by mistake is the costlier error (carnet#1249 D2).
  const isPrivate = audience?.value === 'private';
  const fieldTone = isPrivate ? 'border border-dashed border-warning/60 bg-warning/5' : 'bg-surface-container';
  const placeholder = isListening
    ? t('app.composerListening')
    : (audience?.text.placeholder ?? t('app.composerPlaceholder'));

  return (
    <View
      className="px-3 pt-2 border-t border-border bg-background-primary"
      style={{ borderTopWidth: StyleSheet.hairlineWidth, paddingBottom: composerBottomPadding(keyboardShown, insets.bottom) }}
      testID="chat-input-bar"
    >
      {/* The bar's own popovers, laid out above the field inside the bar's column. */}
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
      {audience ? (
        /*
          The switch and the quote sit over the field rather than in it: the
          phone's field is one 40-tall well, and both controls need a 44-point
          target. Pressing the switch flips it; "Adjust plan from this" flips
          it too and puts the quote beside it. Its name starts with the label
          it shows and carries the state (WCAG 2.5.3), so it announces no
          selected state on top.
        */
        <View className="flex-row items-center" testID="composer-audience-row">
          <Pressable
            onPress={audience.onToggle}
            accessibilityRole="button"
            accessibilityLabel={audience.text.aria}
            className="min-h-11 justify-center"
            testID="composer-audience-switch"
          >
            <View
              className={`flex-row items-center rounded-lg px-2.5 py-1.5 ${isPrivate ? 'bg-warning/15' : 'bg-surface-container'}`}
              testID="composer-audience-chip"
            >
              {isPrivate ? <Ionicons name="lock-closed" size={14} color={colors.ink.warning} /> : null}
              <Text
                className={`text-xs font-semibold ${isPrivate ? 'ml-1 text-on-warning-container' : 'text-text-secondary'}`}
              >
                {audience.text.label}
              </Text>
              <View className="ml-1">
                <Ionicons
                  name="chevron-down"
                  size={14}
                  color={isPrivate ? colors.ink.warning : colors.text.secondary}
                />
              </View>
            </View>
          </Pressable>
          {quote ? (
            <View
              className="ml-2 flex-1 flex-row items-center rounded-lg bg-warning/15 pl-2.5"
              testID="composer-quote-chip"
            >
              <Text numberOfLines={1} className="flex-1 text-xs font-medium text-on-warning-container">
                {quote.label}
              </Text>
              <Pressable
                onPress={quote.onRemove}
                accessibilityRole="button"
                accessibilityLabel={quote.removeLabel}
                className="w-11 h-11 items-center justify-center"
                testID="composer-quote-remove"
              >
                <Ionicons name="close" size={14} color={colors.ink.warning} />
              </Pressable>
            </View>
          ) : null}
        </View>
      ) : null}
      <View className="flex-row items-end">
        <View
          className={`flex-1 min-h-[40px] max-h-[120px] rounded-[20px] ${fieldTone} px-3 justify-center`}
          testID="message-field"
        >
          <TextInput
            ref={inputRef}
            className="text-base text-text-primary py-2"
            placeholder={placeholder}
            placeholderTextColor={isListening ? colors.error : colors.text.tertiary}
            value={displayText}
            onChangeText={onChangeText}
            onSelectionChange={handleSelectionChange}
            onKeyPress={handleKeyPress}
            multiline
            maxLength={4000}
            returnKeyType="default"
            editable={!isListening && !disabled}
            testID="message-input"
          />
        </View>
        {/* The mic and its gap leave together: `VoiceButton` renders null without recognition. */}
        {voiceAvailable && (
          <View className="ml-2">
            <VoiceButton
              isListening={isListening}
              isAvailable={voiceAvailable}
              onPress={onVoicePress}
              disabled={isSending}
              size="sm"
              testID="voice-input-button"
            />
          </View>
        )}
        {/*
          The primary fill and `onPrimary` glyph only when there is something
          to send; otherwise the glyph rests in tertiary ink on nothing. The
          testID flip is what the Maestro flows wait on. While a turn runs the
          slot holds the stop button instead, and a spinner once it was pressed.
        */}
        <TouchableOpacity
          className="w-8 h-8 rounded-full items-center justify-center ml-2"
          style={slot.filled ? { backgroundColor: colors.tokens.primary } : undefined}
          onPress={slot.onPress}
          disabled={slot.disabled}
          accessibilityRole="button"
          accessibilityLabel={slot.label}
          accessibilityState={{ disabled: slot.disabled }}
          testID={slot.testID}
        >
          {isStopping ? (
            <ActivityIndicator size="small" color={colors.text.tertiary} />
          ) : (
            <Ionicons
              name={isSending ? 'stop' : 'arrow-up'}
              size={isSending ? 16 : 18}
              color={slot.filled ? colors.tokens.onPrimary : colors.text.tertiary}
            />
          )}
        </TouchableOpacity>
      </View>
      {isListening && (
        <View className="pt-1 items-center">
          <Text className="text-sm text-error">{t('app.composerTapMicToStop')}</Text>
        </View>
      )}
    </View>
  );
}
