// ABOUTME: Global error boundary that catches unhandled JS exceptions during render
// ABOUTME: Shows a dark-themed fallback UI with Pierre logo and reload button

import React, { Component } from 'react';
import { View, Text, Image, TouchableOpacity } from 'react-native';
import { useTranslation } from '@pierre/i18n';

interface ErrorBoundaryProps {
  children: React.ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

/**
 * The fallback UI, as a function component.
 *
 * The boundary above it has to stay a class — only a class can implement
 * getDerivedStateFromError — but a class cannot call useTranslation, so the
 * copy would have had to stay English. Splitting the view out is the standard
 * way round that, and it keeps the catching and the rendering separate.
 */
function ErrorFallback({ errorText, onReload }: { errorText?: string; onReload: () => void }) {
  const { t } = useTranslation();
  return (

      <View
        testID="error-boundary-fallback"
        className="flex-1 bg-background-primary items-center justify-center px-8"
      >
        <Image
          source={require('../../assets/dravr-logo.png')}
          className="w-20 h-20 mb-8"
          resizeMode="contain"
        />
        <Text className="text-2xl font-bold text-text-primary text-center mb-3">
          {t('app.somethingWentWrong')}
        </Text>
        <Text className="text-base text-text-secondary text-center mb-8">
          {t('app.unexpectedErrorReload')}
        </Text>
        {errorText !== undefined && (
          <View className="bg-background-secondary rounded-lg p-4 mb-8 w-full">
            <Text className="text-sm text-error font-mono" numberOfLines={6}>
              {errorText}
            </Text>
          </View>
        )}
        <TouchableOpacity
          testID="error-boundary-reload"
          onPress={onReload}
          className="bg-primary-500 rounded-lg px-8 py-3"
        >
          <Text className="text-base font-semibold text-on-surface">{t('app.reloadApp')}</Text>
        </TouchableOpacity>
      </View>
  );
}

/**
 * React class component error boundary — hooks cannot catch render errors.
 * Wraps the entire app provider tree to catch:
 * - Unhandled JS exceptions during render
 * - Provider initialization failures
 * - Navigation crashes
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo): void {
    console.error('ErrorBoundary caught an error:', error, errorInfo.componentStack);
  }

  handleReload = (): void => {
    this.setState({ hasError: false, error: null });
  };

  render() {
    if (this.state.hasError) {
      return (
        <ErrorFallback
          errorText={__DEV__ && this.state.error ? this.state.error.toString() : undefined}
          onReload={this.handleReload}
        />
      );
    }

    return this.props.children;
  }
}
