// ABOUTME: Floating search bar component positioned at bottom of screen
// ABOUTME: Keyboard-aware with glass effect per iOS design guidelines, animated on UI thread

import React, { useRef, useEffect, useState } from 'react';
import {
  View,
  TextInput,
  TouchableOpacity,
  ActivityIndicator,
  Keyboard,
  Platform,
  type ViewStyle,
} from 'react-native';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withSpring,
} from 'react-native-reanimated';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { Feather } from '@expo/vector-icons';
import { colors, spacing } from '../../constants/theme';

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
  const insets = useSafeAreaInsets();
  const inputRef = useRef<TextInput>(null);
  const [keyboardHeight, setKeyboardHeight] = useState(0);
  const translateY = useSharedValue(0);

  useEffect(() => {
    const showEvent = Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow';
    const hideEvent = Platform.OS === 'ios' ? 'keyboardWillHide' : 'keyboardDidHide';

    const showSubscription = Keyboard.addListener(showEvent, (e) => {
      const height = e.endCoordinates.height;
      setKeyboardHeight(height);
      translateY.value = withSpring(-height + insets.bottom, {
        damping: 20,
        stiffness: 300,
      });
    });

    const hideSubscription = Keyboard.addListener(hideEvent, () => {
      setKeyboardHeight(0);
      translateY.value = withSpring(0, {
        damping: 20,
        stiffness: 300,
      });
    });

    return () => {
      showSubscription.remove();
      hideSubscription.remove();
    };
  }, [translateY, insets.bottom]);

  const animatedStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: translateY.value }],
  }));

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
          bottom: 0,
          left: 0,
          right: 0,
          paddingBottom: keyboardHeight > 0 ? spacing.sm : spacing.sm,
          paddingTop: spacing.xs,
          paddingHorizontal: spacing.md,
        },
        animatedStyle,
      ]}
      testID={testID ? `${testID}-container` : undefined}
    >
      <View
        className="flex-row items-center rounded-full px-4 min-h-[44px]"
        style={{
          backgroundColor: 'rgba(30, 30, 46, 0.92)',
          borderColor: 'rgba(139, 92, 246, 0.4)',
          borderWidth: 1,
          borderRadius: 9999,
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
          <ActivityIndicator size="small" color={colors.primary[500]} />
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
