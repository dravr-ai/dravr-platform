// ABOUTME: Reusable text input component with dark theme and glass styling per Stitch design
// ABOUTME: Supports labels, error states, password visibility toggle, and glass variant

import React, { useState } from 'react';
import {
  View,
  TextInput,
  Text,
  TouchableOpacity,
  type TextInputProps,
  type TextStyle,
  type ViewStyle,
} from 'react-native';
import { BOREAL_LIGHT, PRIMARY_PALETTE, useThemeColors } from '../../constants/theme';

interface InputProps extends Omit<TextInputProps, 'style'> {
  label?: string;
  error?: string;
  containerStyle?: ViewStyle;
  showPasswordToggle?: boolean;
  variant?: 'default' | 'glass';
  /**
   * Force the input to render with the BOREAL_LIGHT palette regardless of
   * the user's appearance preference. Used by always-light brand surfaces
   * (e.g. the login card sitting on the deep-green hero) where the parent
   * background is fixed and the input must remain legible against it.
   */
  surface?: 'auto' | 'light';
  testID?: string;
}

export function Input({
  label,
  error,
  containerStyle,
  showPasswordToggle = false,
  variant = 'default',
  surface = 'auto',
  secureTextEntry,
  testID,
  ...props
}: InputProps) {
  const themeColors = useThemeColors();
  const [isPasswordVisible, setIsPasswordVisible] = useState(false);

  const shouldHidePassword = secureTextEntry && !isPasswordVisible;
  const isLightSurface = surface === 'light';

  // When pinned to light, use BOREAL_LIGHT tokens directly so the input reads
  // correctly on a fixed white card even when the global theme is dark.
  const fieldColors = isLightSurface
    ? {
        text: BOREAL_LIGHT.onSurface,
        secondary: BOREAL_LIGHT.onSurfaceVariant,
        tertiary: BOREAL_LIGHT.outline,
        background: BOREAL_LIGHT.surfaceContainerLow,
        border: 'rgba(26, 28, 27, 0.10)',
        errorBorder: BOREAL_LIGHT.error,
        accent: BOREAL_LIGHT.primary,
        errorText: BOREAL_LIGHT.error,
      }
    : {
        text: themeColors.text.primary,
        secondary: themeColors.text.secondary,
        tertiary: themeColors.text.tertiary,
        background: themeColors.background.secondary,
        border: themeColors.border.default,
        errorBorder: themeColors.error,
        accent: themeColors.text.accent,
        errorText: themeColors.error,
      };

  const inputBaseStyle: ViewStyle & TextStyle = {
    flex: 1,
    paddingVertical: 10,
    paddingHorizontal: 16,
    borderRadius: variant === 'glass' ? 16 : 12,
    borderWidth: 1,
    borderColor: error ? fieldColors.errorBorder : fieldColors.border,
    backgroundColor:
      variant === 'glass' && !isLightSurface
        ? 'rgba(255, 255, 255, 0.03)'
        : fieldColors.background,
    color: fieldColors.text,
    fontSize: 16,
    paddingRight: showPasswordToggle ? 64 : undefined,
  };

  return (
    <View className="mb-4" style={containerStyle}>
      {label && (
        <Text
          className="text-sm mb-1 font-medium"
          style={{ color: fieldColors.secondary }}
        >
          {label}
        </Text>
      )}
      <View className="relative flex-row items-center">
        <TextInput
          style={inputBaseStyle}
          placeholderTextColor={fieldColors.tertiary}
          selectionColor={PRIMARY_PALETTE[500]}
          secureTextEntry={shouldHidePassword}
          testID={testID}
          {...props}
        />
        {showPasswordToggle && secureTextEntry !== undefined && (
          <TouchableOpacity
            className="absolute right-4 py-1"
            onPress={() => setIsPasswordVisible(!isPasswordVisible)}
          >
            <Text
              className="text-sm font-medium"
              style={{ color: fieldColors.accent }}
            >
              {isPasswordVisible ? 'Hide' : 'Show'}
            </Text>
          </TouchableOpacity>
        )}
      </View>
      {error && (
        <Text className="text-xs mt-1" style={{ color: fieldColors.errorText }}>
          {error}
        </Text>
      )}
    </View>
  );
}
