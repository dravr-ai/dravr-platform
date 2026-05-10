// ABOUTME: Floating search bar component positioned at bottom of screen
// ABOUTME: Keyboard-aware with glass effect per iOS design guidelines, animated with keyboard

import React, { useRef, useEffect } from 'react';
import {
  View,
  TextInput,
  TouchableOpacity,
  ActivityIndicator,
  Keyboard,
  Platform,
  Animated,
  type ViewStyle,
} from 'react-native';
import { Feather } from '@expo/vector-icons';
import { PRIMARY_PALETTE, spacing, useThemeColors, useTheme } from '../../constants/theme';
import { TAB_BAR_BOTTOM_OFFSET } from './ExpandableTabBar';

// Transparent container — only the inner pill is visible
const containerStyle: ViewStyle = {
  backgroundColor: 'transparent',
};

interface FloatingSearchBarProps {
  value: string;
  onChangeText: (text: string) => void;
  onSubmit?: () => void;
  placeholder?: string;
  isSearching?: boolean;
  testID?: string;
  autoFocus?: boolean;
}

export function FloatingSearchBar({
  value,
  onChangeText,
  onSubmit,
  placeholder = 'Search...',
  isSearching = false,
  testID,
  autoFocus = false,
}: FloatingSearchBarProps) {
  const colors = useThemeColors();
  const { scheme } = useTheme();
  const isDark = scheme === 'dark';
  const inputRef = useRef<TextInput>(null);
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

    return () => {
      showSub.remove();
      hideSub.remove();
    };
  }, [bottomAnim]);

  const handleClear = () => {
    onChangeText('');
    inputRef.current?.focus();
  };

  const handleSubmit = () => {
    Keyboard.dismiss();
    onSubmit?.();
  };

  return (
    <Animated.View
      style={[
        containerStyle,
        {
          position: 'absolute',
          bottom: bottomAnim,
          left: 0,
          right: 0,
          paddingBottom: spacing.sm,
          paddingTop: spacing.xs,
          paddingHorizontal: spacing.md,
        },
      ]}
      testID={testID ? `${testID}-container` : undefined}
    >
      <View
        className="flex-row items-center rounded-full px-4 min-h-[44px]"
        style={{
          backgroundColor: isDark ? colors.background.elevated : colors.background.primary,
          borderColor: isDark ? 'rgba(192, 200, 195, 0.18)' : 'rgba(26, 28, 27, 0.10)',
          borderWidth: 1,
          borderRadius: 9999,
          shadowColor: isDark ? '#000000' : '#1a1c1b',
          shadowOffset: { width: 0, height: 4 },
          shadowOpacity: isDark ? 0.4 : 0.06,
          shadowRadius: 12,
          elevation: 4,
        }}
      >
        <Feather name="search" size={18} color={colors.text.tertiary} />
        <TextInput
          ref={inputRef}
          testID={testID}
          className="flex-1 ml-2 text-text-primary text-base"
          placeholder={placeholder}
          placeholderTextColor={colors.text.tertiary}
          value={value}
          onChangeText={onChangeText}
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="search"
          onSubmitEditing={handleSubmit}
          autoFocus={autoFocus}
        />
        {isSearching ? (
          <ActivityIndicator size="small" color={PRIMARY_PALETTE[500]} />
        ) : value.length > 0 ? (
          <TouchableOpacity
            onPress={handleClear}
            hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
          >
            <Feather name="x" size={18} color={colors.text.tertiary} />
          </TouchableOpacity>
        ) : null}
      </View>
    </Animated.View>
  );
}
