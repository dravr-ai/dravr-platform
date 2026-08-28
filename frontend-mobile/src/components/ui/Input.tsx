// ABOUTME: Boreal Editorial Input — bottom-stroke underline, DESIGN.md §5
// ABOUTME: Matches the web Input so one component reads the same on both platforms

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
import { useTranslation } from '@pierre/i18n';

interface InputProps extends Omit<TextInputProps, 'style'> {
  label?: string;
  error?: string;
  containerStyle?: ViewStyle;
  showPasswordToggle?: boolean;
  /**
   * Retained for API stability. The Boreal system ships one editorial
   * underline, so both values render identically — same as the web Input.
   */
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
  variant: _variant = 'default',
  surface = 'auto',
  secureTextEntry,
  testID,
  ...props
}: InputProps) {
  const { t } = useTranslation();
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

  // Boreal Editorial field, DESIGN.md §5 — the same single bottom stroke the web
  // Input wears. The two platforms rendered the same component in two languages
  // (filled box here, underline there) while DESIGN.md described one Product
  // Tier covering both.
  //
  // Touch is respected by padding, not by a box: 12pt vertical against a 16pt
  // font clears the 44pt minimum target without an enclosing rectangle.
  const inputBaseStyle: ViewStyle & TextStyle = {
    flex: 1,
    paddingVertical: 12,
    paddingHorizontal: 0,
    borderRadius: 0,
    borderWidth: 0,
    borderBottomWidth: 1,
    borderBottomColor: error ? fieldColors.errorBorder : fieldColors.border,
    backgroundColor: 'transparent',
    color: fieldColors.text,
    fontSize: 16,
    paddingRight: showPasswordToggle ? 64 : undefined,
  };

  return (
    <View className="mb-4" style={containerStyle}>
      {label && (
        <Text
          className="text-[11px] mb-2 font-medium uppercase"
          style={{ color: fieldColors.secondary, letterSpacing: 0.08 * 11 }}
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
              {isPasswordVisible ? t('app.hide') : t('app.show')}
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
