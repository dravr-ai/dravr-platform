// ABOUTME: Unit tests for ErrorBoundary component
// ABOUTME: Verifies fallback UI on error, normal rendering, and reload behavior

import React from 'react';
import { render, fireEvent } from '@testing-library/react-native';
import { Text } from 'react-native';
import { ErrorBoundary } from '../src/components/ErrorBoundary';

// Suppress console.error from componentDidCatch during tests
const originalConsoleError = console.error;
beforeEach(() => {
  console.error = jest.fn();
});
afterEach(() => {
  console.error = originalConsoleError;
});

function ProblemChild(): React.JSX.Element {
  throw new Error('Test render error');
}

function GoodChild(): React.JSX.Element {
  return <Text>All good</Text>;
}

describe('ErrorBoundary', () => {
  it('should render children when no error occurs', () => {
    const { getByText } = render(
      <ErrorBoundary>
        <GoodChild />
      </ErrorBoundary>
    );

    expect(getByText('All good')).toBeTruthy();
  });

  it('should show fallback UI when a child throws during render', () => {
    const { getByText, getByTestId } = render(
      <ErrorBoundary>
        <ProblemChild />
      </ErrorBoundary>
    );

    expect(getByTestId('error-boundary-fallback')).toBeTruthy();
    expect(getByText('Something went wrong')).toBeTruthy();
    expect(getByText('Reload App')).toBeTruthy();
  });

  it('should log the error via componentDidCatch', () => {
    render(
      <ErrorBoundary>
        <ProblemChild />
      </ErrorBoundary>
    );

    expect(console.error).toHaveBeenCalledWith(
      'ErrorBoundary caught an error:',
      expect.any(Error),
      expect.any(String)
    );
  });

  it('should show error details in dev mode', () => {
    const { getByText } = render(
      <ErrorBoundary>
        <ProblemChild />
      </ErrorBoundary>
    );

    // __DEV__ is true in test environment
    expect(getByText(/Test render error/)).toBeTruthy();
  });

  it('should reset error state when reload button is pressed', () => {
    let shouldThrow = true;

    function ConditionalChild(): React.JSX.Element {
      if (shouldThrow) {
        throw new Error('Conditional error');
      }
      return <Text>Recovered</Text>;
    }

    const { getByTestId, getByText } = render(
      <ErrorBoundary>
        <ConditionalChild />
      </ErrorBoundary>
    );

    expect(getByTestId('error-boundary-fallback')).toBeTruthy();

    // Fix the issue before reloading
    shouldThrow = false;

    fireEvent.press(getByTestId('error-boundary-reload'));

    expect(getByText('Recovered')).toBeTruthy();
  });
});
