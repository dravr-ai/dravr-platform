// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Error boundary component to catch and handle React render errors gracefully
// ABOUTME: Prevents entire app crashes when a component throws during rendering

import { Component, type ErrorInfo, type ReactNode } from 'react';
import { AlertTriangle, RefreshCw, Home } from 'lucide-react';
import { i18n } from '@pierre/i18n';

interface ErrorBoundaryProps {
  children: ReactNode;
  /** Custom fallback UI to render when an error occurs */
  fallback?: ReactNode;
  /** Called when an error is caught */
  onError?: (error: Error, errorInfo: ErrorInfo) => void;
  /** Whether to show a "Go Home" button (useful for nested boundaries) */
  showHomeButton?: boolean;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
  errorInfo: ErrorInfo | null;
}

/**
 * Error Boundary component that catches JavaScript errors anywhere in the child
 * component tree, logs those errors, and displays a fallback UI.
 *
 * Note: Error boundaries do NOT catch errors for:
 * - Event handlers (use try/catch)
 * - Asynchronous code (e.g., setTimeout callbacks)
 * - Server-side rendering
 * - Errors thrown in the error boundary itself
 *
 * @example
 * // Wrap entire app
 * <ErrorBoundary>
 *   <App />
 * </ErrorBoundary>
 *
 * @example
 * // Wrap specific feature with custom error handler
 * <ErrorBoundary
 *   onError={(error) => logErrorToService(error)}
 *   fallback={<CustomErrorFallback />}
 * >
 *   <RiskyFeature />
 * </ErrorBoundary>
 */
class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = {
      hasError: false,
      error: null,
      errorInfo: null,
    };
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    // Update state so the next render will show the fallback UI
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    // Auto-reload on stale chunk errors (deploy happened while user had old HTML cached)
    if (
      error.message.includes('Failed to fetch dynamically imported module') ||
      error.message.includes('Loading chunk') ||
      error.message.includes('Loading CSS chunk')
    ) {
      window.location.reload();
      return;
    }

    // Log the error to console
    console.error('ErrorBoundary caught an error:', error);
    console.error('Component stack:', errorInfo.componentStack);

    // Update state with error info
    this.setState({ errorInfo });

    // Call custom error handler if provided
    if (this.props.onError) {
      this.props.onError(error, errorInfo);
    }
  }

  handleRetry = (): void => {
    this.setState({
      hasError: false,
      error: null,
      errorInfo: null,
    });
  };

  handleGoHome = (): void => {
    // Clear error state and navigate to home
    this.setState({
      hasError: false,
      error: null,
      errorInfo: null,
    });
    window.location.href = '/';
  };

  render(): ReactNode {
    const { hasError, error, errorInfo } = this.state;
    const { children, fallback, showHomeButton = true } = this.props;

    if (hasError) {
      // Return custom fallback if provided
      if (fallback) {
        return fallback;
      }

      // Default error UI
      return (
        <div className="min-h-dvh bg-surface flex items-center justify-center px-4">
          <div className="max-w-lg w-full">
            {/* Error Card */}
            <div className="bg-surface-container-low rounded-xl overflow-hidden border border-on-surface">
              {/* Header with gradient */}
              <div className="h-1.5 w-full bg-error" />

              <div className="px-8 py-10">
                {/* Error Icon */}
                <div className="flex justify-center mb-6">
                  <div className="p-4 bg-error/10 rounded-full">
                    <AlertTriangle className="w-12 h-12 text-error" />
                  </div>
                </div>

                {/* Error Title */}
                <h1 className="text-2xl font-bold text-on-surface text-center mb-3">
                  {i18n.t('shell.errorTitle')}
                </h1>

                {/* Error Description */}
                <p className="text-outline text-center mb-6">
                  {i18n.t('shell.errorUnexpectedBody')}
                </p>

                {/* Error Details (collapsible in production) */}
                {error && (
                  <details className="mb-6 bg-surface-container-highest/50 rounded-lg overflow-hidden">
                    <summary className="px-4 py-3 text-sm text-outline cursor-pointer hover:bg-surface-container-highest/70 transition-colors">
                      {i18n.t('shell.errorTechnicalDetails')}
                    </summary>
                    <div className="px-4 py-3 border-t border-on-surface">
                      <p className="text-sm font-mono text-error mb-2">
                        {error.name}: {error.message}
                      </p>
                      {errorInfo?.componentStack && (
                        <pre className="text-xs text-on-surface-variant overflow-x-auto max-h-32 scrollbar-thin">
                          {errorInfo.componentStack}
                        </pre>
                      )}
                    </div>
                  </details>
                )}

                {/* Action Buttons */}
                <div className="flex flex-col sm:flex-row gap-3">
                  <button
                    onClick={this.handleRetry}
                    className="flex-1 flex items-center justify-center gap-2 px-4 py-3 bg-primary hover:bg-primary/90 text-on-primary font-medium rounded-lg transition-colors"
                  >
                    <RefreshCw className="w-4 h-4" />
                    {i18n.t('chat.tryAgain')}
                  </button>

                  {showHomeButton && (
                    <button
                      onClick={this.handleGoHome}
                      className="flex-1 flex items-center justify-center gap-2 px-4 py-3 bg-surface-container hover:bg-on-surface-variant text-on-surface font-medium rounded-lg transition-colors"
                    >
                      <Home className="w-4 h-4" />
                      {i18n.t('shell.errorGoHome')}
                    </button>
                  )}
                </div>
              </div>

              {/* Footer */}
              <div className="px-8 py-4 bg-surface-container-highest/30 border-t border-on-surface">
                <p className="text-xs text-on-surface-variant text-center">
                  {i18n.t('shell.errorPersistHint')}
                </p>
              </div>
            </div>
          </div>
        </div>
      );
    }

    return children;
  }
}

export default ErrorBoundary;
