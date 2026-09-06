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
import { PRIMARY_PALETTE, useThemeColors } from '../../constants/theme';
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
  testID?: string;
}

export function Input({
  label,
  error,
  containerStyle,
  showPasswordToggle = false,
  variant: _variant = 'default',
  secureTextEntry,
  testID,
  ...props
}: InputProps) {
  const { t } = useTranslation();
  const themeColors = useThemeColors();
  const [isPasswordVisible, setIsPasswordVisible] = useState(false);

  const shouldHidePassword = secureTextEntry && !isPasswordVisible;

  // The live palette, always. There used to be a `surface="light"` escape
  // hatch that pinned these to BOREAL_LIGHT for a screen whose background was
  // a fixed deep-green hero; that hero is gone and every surface follows the
  // athlete's appearance setting, so the escape hatch had no caller left.
  const fieldColors = {
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
      {/* Sentence case at the label step, no tracking — DESIGN.md §3. The 11px
          caps at 0.08em were the v1 label face; web retired them in the v2 token
          pass and the phone kept them, so the same form read as two products
          depending on which client an athlete opened. */}
      {label && (
        <Text
          className="text-sm mb-2 font-medium"
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
