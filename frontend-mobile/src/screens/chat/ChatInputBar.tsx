// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat input bar — the "/" button, the slash-command and @handle palettes, voice and send
// ABOUTME: Keyboard-aware positioning — animates above keyboard or tab bar

import React, { useEffect, useRef, useState } from 'react';
import { View, TextInput, TouchableOpacity, ActivityIndicator, Text, Animated } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { COMMAND_PREFIX, isCommandDraft } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { spacing, useThemeColors, useTheme } from '../../constants/theme';
import { VoiceButton } from '../../components/ui';
import { CommandPalette } from '../../components/CommandPalette';
import { MentionPalette } from '../../components/MentionPalette';
import { useCommandPalette } from '../../hooks/useCommandPalette';
import { useMentionPalette } from '../../hooks/useMentionPalette';
import { COMPOSER_KEYS, composerKey, type ComposerKeyEvent } from '../../hooks/composerKeys';

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
  /** Where the composer sits with the keyboard closed, safe-area included. */
  restingOffset: number;
  /** Keyboard height in dp, 0 when closed. */
  keyboardHeight: number;
  /** The OS keyboard animation duration, so the composer moves with it. */
  keyboardDuration: number;
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
  restingOffset,
  keyboardHeight,
  keyboardDuration,
}: ChatInputBarProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { scheme } = useTheme();
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
  const canSend = inputText.trim() && !isSending && !isListening && !disabled;

  /**
   * Hardware-keyboard keys, offered to the command palette first and the
   * mention palette second — only one of the two is ever open. What neither
   * takes falls through to the field, except Enter on a finished command:
   * the athlete typed the whole thing and means to send it.
   */
  const handleKeyPress = (event: ComposerKeyEvent) => {
    if (palette.handleKeyPress(event)) return;
    if (mentions.handleKeyPress(event)) return;
    if (composerKey(event) === COMPOSER_KEYS.enter && canSend && isCommandDraft(inputText)) {
      onSendMessage();
    }
  };

  /** The visible way in to the palette, the way Telegram's bot menu button is. */
  const openCommandPalette = () => {
    onChangeText(COMMAND_PREFIX);
    inputRef.current?.focus();
  };
  const isDark = scheme === 'dark';

  // Composer pill matches the surrounding canvas — elevated lowest tier with
  // a hairline outline-variant edge in both schemes. The accent ring uses the
  // active primary so the input reads as the primary action surface.
  const pillBackground = isDark ? colors.background.elevated : colors.background.primary;
  const pillBorder = isDark
    ? 'rgba(192, 200, 195, 0.18)'
    : 'rgba(26, 28, 27, 0.10)';

  // How far the composer must rise above its resting place. `bottom` stays
  // fixed and the movement is a transform, because `bottom` cannot be animated
  // on the native driver: the old version ran useNativeDriver:false and drove a
  // LAYOUT property from JS on every keyboard frame, which is the textbook way
  // to drop frames on the exact interaction the app is built around.
  const rise = Math.max(0, keyboardHeight - restingOffset);
  const riseAnim = useRef(new Animated.Value(0)).current;

  useEffect(() => {
    Animated.timing(riseAnim, {
      toValue: -rise,
      duration: keyboardDuration,
      useNativeDriver: true,
    }).start();
  }, [rise, keyboardDuration, riseAnim]);

  return (
    <Animated.View
      style={{
        position: 'absolute',
        bottom: restingOffset,
        transform: [{ translateY: riseAnim }],
        left: 0,
        right: 0,
        paddingHorizontal: spacing.md,
        paddingVertical: spacing.xs,
        backgroundColor: 'transparent',
      }}
    >
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
      <View
        className="flex-row items-center rounded-full px-3 min-h-[44px] max-h-[100px]"
        style={{
          backgroundColor: pillBackground,
          borderColor: pillBorder,
          borderWidth: 1,
          borderRadius: 9999,
          shadowColor: isDark ? '#000000' : '#1a1c1b',
          shadowOffset: { width: 0, height: 4 },
          shadowOpacity: isDark ? 0.4 : 0.06,
          shadowRadius: 12,
          elevation: 4,
        }}
      >
        <TouchableOpacity
          className="w-9 h-9 rounded-full items-center justify-center mr-1"
          style={{ backgroundColor: `${colors.pierre.violet}1F` }}
          onPress={openCommandPalette}
          disabled={isListening || disabled}
          accessibilityRole="button"
          accessibilityLabel={t('app.composerCommandsAria')}
          testID="slash-command-button"
        >
          <Text className="text-lg font-bold" style={{ color: colors.pierre.violet }}>
            {COMMAND_PREFIX}
          </Text>
        </TouchableOpacity>
        <TextInput
          ref={inputRef}
          className="flex-1 text-base text-text-primary py-2 max-h-[100px]"
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
        <VoiceButton
          isListening={isListening}
          isAvailable={voiceAvailable}
          onPress={onVoicePress}
          disabled={isSending}
          size="sm"
          testID="voice-input-button"
        />
        {/* Violet send button per Stitch spec */}
        <TouchableOpacity
          className={`w-9 h-9 rounded-full items-center justify-center ml-2 ${
            !canSend ? 'bg-background-tertiary' : ''
          }`}
          style={canSend ? { backgroundColor: colors.pierre.violet } : undefined}
          onPress={onSendMessage}
          disabled={!canSend}
          accessibilityRole="button"
          accessibilityLabel={t('app.composerSendAria')}
          accessibilityState={{ disabled: !canSend }}
          testID={canSend ? 'send-button' : 'send-button-disabled'}
        >
          {isSending ? (
            <ActivityIndicator size="small" color={canSend ? colors.tokens.onPrimary : colors.text.tertiary} />
          ) : (
            <Ionicons
              name="arrow-up"
              size={20}
              color={canSend ? colors.tokens.onPrimary : colors.text.tertiary}
            />
          )}
        </TouchableOpacity>
      </View>
      {isListening && (
        <View className="pt-1 items-center">
          <Text className="text-xs text-error">{t('app.composerTapMicToStop')}</Text>
        </View>
      )}
    </Animated.View>
  );
}
