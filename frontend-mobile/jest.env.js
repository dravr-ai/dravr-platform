// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Jest stand-in for Expo's virtual env module, which babel-preset-expo compiles EXPO_PUBLIC_* reads into
// ABOUTME: Serves the real process.env so a build-time flag is readable from a test the way it is from a build

module.exports = { env: process.env };
