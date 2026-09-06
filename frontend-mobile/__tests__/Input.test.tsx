// ABOUTME: Unit tests for Input component
// ABOUTME: Tests labels, errors, password toggle, and input behavior

import React from 'react';
import { render, fireEvent } from '@testing-library/react-native';
import { Input } from '../src/components/ui/Input';

describe('Input Component', () => {
  describe('rendering', () => {
    it('should render basic input', () => {
      const { getByTestId } = render(
        <Input testID="basic-input" placeholder="Enter text" />
      );
      expect(getByTestId('basic-input')).toBeTruthy();
    });

    it('should render with placeholder', () => {
      const { getByPlaceholderText } = render(
        <Input placeholder="Enter your name" />
      );
      expect(getByPlaceholderText('Enter your name')).toBeTruthy();
    });
  });

  describe('label', () => {
    it('should render label when provided', () => {
      const { getByText } = render(
        <Input label="Email Address" placeholder="Enter email" />
      );
      expect(getByText('Email Address')).toBeTruthy();
    });

    it('should not render label when not provided', () => {
      const { queryByText } = render(
        <Input placeholder="No label" />
      );
      expect(queryByText('Email Address')).toBeNull();
    });

    // DESIGN.md §3: labels are sentence case in the body face, no tracking.
    // The 11px caps at 0.08em were the v1 label face, retired on web in the
    // Boreal v2 token pass; the phone kept them for months and nothing here
    // noticed, because `uppercase` is a NativeWind class with no compiled CSS
    // under jest — the rendered TEXT is unchanged either way, so an assertion
    // on the words alone cannot see the regression. Assert the class and the
    // style, which is what actually differs.
    it('sets the label in sentence case at the body step, never tracked caps', () => {
      const { getByText } = render(<Input label="Email address" placeholder="x" />);
      const label = getByText('Email address');

      expect(label.props.className).toContain('text-sm');
      expect(label.props.className).not.toMatch(/uppercase/);
      expect(label.props.className).not.toMatch(/text-\[11px\]/);

      // Tracking is the other half of the retired face: the old label set
      // letterSpacing to 0.08em of 11px. Any positive tracking here is that
      // face coming back.
      const raw = label.props.style as { letterSpacing?: number } | Array<{ letterSpacing?: number }>;
      const layers = Array.isArray(raw) ? raw : [raw];
      const tracking = layers.reduce<number | undefined>(
        (found, layer) => layer?.letterSpacing ?? found,
        undefined,
      );
      expect(tracking).toBeUndefined();
    });
  });

  describe('error state', () => {
    it('should render error message when provided', () => {
      const { getByText } = render(
        <Input placeholder="Enter email" error="Invalid email format" />
      );
      expect(getByText('Invalid email format')).toBeTruthy();
    });

    it('should not render error when not provided', () => {
      const { queryByText } = render(
        <Input placeholder="No error" />
      );
      expect(queryByText('Invalid email format')).toBeNull();
    });

    it('should show both label and error', () => {
      const { getByText } = render(
        <Input
          label="Email"
          placeholder="Enter email"
          error="This field is required"
        />
      );
      expect(getByText('Email')).toBeTruthy();
      expect(getByText('This field is required')).toBeTruthy();
    });
  });

  describe('text input behavior', () => {
    it('should accept text input', () => {
      const onChangeMock = jest.fn();
      const { getByTestId } = render(
        <Input
          testID="text-input"
          onChangeText={onChangeMock}
          placeholder="Type here"
        />
      );

      fireEvent.changeText(getByTestId('text-input'), 'Hello World');
      expect(onChangeMock).toHaveBeenCalledWith('Hello World');
    });

    it('should handle value prop', () => {
      const { getByDisplayValue } = render(
        <Input value="Initial Value" placeholder="Enter text" />
      );
      expect(getByDisplayValue('Initial Value')).toBeTruthy();
    });
  });

  describe('password toggle', () => {
    it('should show toggle when showPasswordToggle is true and secureTextEntry is set', () => {
      const { getByText } = render(
        <Input
          placeholder="Password"
          secureTextEntry={true}
          showPasswordToggle={true}
        />
      );
      expect(getByText('Show')).toBeTruthy();
    });

    it('should toggle password visibility when pressed', () => {
      const { getByText } = render(
        <Input
          placeholder="Password"
          secureTextEntry={true}
          showPasswordToggle={true}
        />
      );

      // Initially shows "Show"
      const toggleButton = getByText('Show');
      expect(toggleButton).toBeTruthy();

      // Press to show password
      fireEvent.press(toggleButton);
      expect(getByText('Hide')).toBeTruthy();

      // Press again to hide password
      fireEvent.press(getByText('Hide'));
      expect(getByText('Show')).toBeTruthy();
    });

    it('should not show toggle when showPasswordToggle is false', () => {
      const { queryByText } = render(
        <Input
          placeholder="Password"
          secureTextEntry={true}
          showPasswordToggle={false}
        />
      );
      expect(queryByText('Show')).toBeNull();
      expect(queryByText('Hide')).toBeNull();
    });
  });

  describe('custom styles', () => {
    it('should accept containerStyle prop', () => {
      const { getByTestId } = render(
        <Input
          testID="styled-input"
          containerStyle={{ marginTop: 20 }}
          placeholder="Styled"
        />
      );
      expect(getByTestId('styled-input')).toBeTruthy();
    });
  });

  describe('input props passthrough', () => {
    it('should pass through keyboardType', () => {
      const { getByTestId } = render(
        <Input
          testID="email-input"
          keyboardType="email-address"
          placeholder="Email"
        />
      );
      expect(getByTestId('email-input').props.keyboardType).toBe('email-address');
    });

    it('should pass through autoCapitalize', () => {
      const { getByTestId } = render(
        <Input
          testID="name-input"
          autoCapitalize="words"
          placeholder="Name"
        />
      );
      expect(getByTestId('name-input').props.autoCapitalize).toBe('words');
    });

    it('should pass through maxLength', () => {
      const { getByTestId } = render(
        <Input
          testID="limited-input"
          maxLength={100}
          placeholder="Limited"
        />
      );
      expect(getByTestId('limited-input').props.maxLength).toBe(100);
    });
  });
});
