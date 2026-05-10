// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat input bar component with text input, voice, and send buttons
// ABOUTME: Keyboard-aware positioning — animates above keyboard or tab bar

import React, { useEffect, useRef } from 'react';
import { View, TextInput, TouchableOpacity, ActivityIndicator, Text, Keyboard, Platform, Animated } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { spacing, useThemeColors, useTheme } from '../../constants/theme';
import { VoiceButton, TAB_BAR_BOTTOM_OFFSET } from '../../components/ui';

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
  const colors = useThemeColors();
  const { scheme } = useTheme();
  const displayText = isListening ? partialTranscript : inputText;
  const canSend = inputText.trim() && !isSending && !isListening && !disabled;
  const isDark = scheme === 'dark';

  // Composer pill matches the surrounding canvas — elevated lowest tier with
  // a hairline outline-variant edge in both schemes. The accent ring uses the
  // active primary so the input reads as the primary action surface.
  const pillBackground = isDark ? colors.background.elevated : colors.background.primary;
  const pillBorder = isDark
    ? 'rgba(192, 200, 195, 0.18)'
    : 'rgba(26, 28, 27, 0.10)';

  const bottomAnim = useRef(new Animated.Value(TAB_BAR_BOTTOM_OFFSET)).current;

  useEffect(() => {
    const showEvent = Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow';
    const hideEvent = Platform.OS === 'ios' ? 'keyboardWillHide' : 'keyboardDidHide';

    const showSub = Keyboard.addListener(showEvent, (e) => {
      Animated.timing(bottomAnim, {
        toValue: e.endCoordinates.height,
        duration: Platform.OS === 'ios' ? e.duration : 250,
        useNativeDriver: false,
      }).start();
    });

    const hideSub = Keyboard.addListener(hideEvent, (e) => {
      Animated.timing(bottomAnim, {
        toValue: TAB_BAR_BOTTOM_OFFSET,
        duration: Platform.OS === 'ios' ? (e.duration ?? 250) : 250,
        useNativeDriver: false,
      }).start();
    });

    return () => { showSub.remove(); hideSub.remove(); };
  }, [bottomAnim]);

  return (
    <Animated.View
      style={{
        position: 'absolute',
        bottom: bottomAnim,
        left: 0,
        right: 0,
        paddingHorizontal: spacing.md,
        paddingVertical: spacing.xs,
        backgroundColor: 'transparent',
      }}
    >
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
        <TextInput
          ref={inputRef}
          className="flex-1 text-base text-text-primary py-2 max-h-[100px]"
          placeholder={isListening ? 'Listening...' : 'Message Dravr...'}
          placeholderTextColor={isListening ? colors.error : colors.text.tertiary}
          value={displayText}
          onChangeText={onChangeText}
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
          <Text className="text-xs text-error">Tap mic to stop recording</Text>
        </View>
      )}
    </Animated.View>
  );
}
