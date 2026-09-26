// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat composer bar — the field, the slash-command and @handle palettes, voice and send
// ABOUTME: An in-flow bar under the thread: the layout, not a transform, keeps it above the keyboard

import React, { useEffect, useState } from 'react';
import { View, TextInput, TouchableOpacity, ActivityIndicator, Text, StyleSheet, Keyboard, Platform } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { Ionicons } from '@expo/vector-icons';
import { isCommandDraft } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { PALETTE_KEYS } from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import { VoiceButton } from '../../components/ui';
import { CommandPalette } from '../../components/CommandPalette';
import { MentionPalette } from '../../components/MentionPalette';
import { useCommandPalette } from '../../hooks/useCommandPalette';
import { useMentionPalette } from '../../hooks/useMentionPalette';
import { composerKey, type ComposerKeyEvent } from '../../hooks/composerKeys';

interface ChatInputBarProps {
  inputText: string;
  partialTranscript: string;
  isListening: boolean;
  isSending: boolean;
  /** When true, input and send are disabled (e.g. usage quota blocked) */
  disabled?: boolean;
  voiceAvailable: boolean;
  inputRef: React.RefObject<TextInput | null>;
  onChangeText: (text: string) => void;
  onVoicePress: () => void;
  onSendMessage: () => void;
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
  disabled = false,
  voiceAvailable,
  inputRef,
  onChangeText,
  onVoicePress,
  onSendMessage,
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

  const sendInk = canSend ? colors.tokens.onPrimary : colors.text.tertiary;

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
      <View className="flex-row items-end">
        <View
          className="flex-1 min-h-[40px] max-h-[120px] rounded-[20px] bg-surface-container px-3 justify-center"
          testID="message-field"
        >
          <TextInput
            ref={inputRef}
            className="text-base text-text-primary py-2"
            placeholder={isListening ? t('app.composerListening') : t('app.composerPlaceholder')}
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
          testID flip is what the Maestro flows wait on.
        */}
        <TouchableOpacity
          className="w-8 h-8 rounded-full items-center justify-center ml-2"
          style={canSend ? { backgroundColor: colors.tokens.primary } : undefined}
          onPress={onSendMessage}
          disabled={!canSend}
          accessibilityRole="button"
          accessibilityLabel={t('app.composerSendAria')}
          accessibilityState={{ disabled: !canSend }}
          testID={canSend ? 'send-button' : 'send-button-disabled'}
        >
          {isSending ? (
            <ActivityIndicator size="small" color={sendInk} />
          ) : (
            <Ionicons name="arrow-up" size={18} color={sendInk} />
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
