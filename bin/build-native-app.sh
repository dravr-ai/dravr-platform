#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Builds and installs the native Pierre app on the iOS Simulator via Xcode
# ABOUTME: Required only for testing native modules (speech recognition, native MMKV)

set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

EXPO_PORT="${EXPO_PORT:-8082}"

# Parse args
NO_BUNDLER=false
for arg in "$@"; do
    case $arg in
        --no-bundler)
            NO_BUNDLER=true
            shift
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=ios-simulator.sh
. "$SCRIPT_DIR/ios-simulator.sh"

# IOS_SIM_UDID picks the device when more than one is booted.
BOOTED_UDID="${IOS_SIM_UDID:-$(ios_sim_booted_udid)}"

if [ -z "$BOOTED_UDID" ]; then
    echo -e "${YELLOW}No iOS Simulator is booted. Start one first:${NC}"
    ios_sim_boot_hint
    exit 1
fi

SIM_NAME="$(ios_sim_device_name "$BOOTED_UDID")"

echo "Building native app for $SIM_NAME ($BOOTED_UDID)..."
echo -e "${YELLOW}This requires Xcode and may take several minutes on first build.${NC}"

cd "$PROJECT_ROOT/frontend-mobile"

if [ "$NO_BUNDLER" = "true" ]; then
    echo "Building without bundler (Metro must be running separately)..."
    npx expo run:ios --no-bundler --device "$BOOTED_UDID"
else
    echo "Building and starting Metro on port $EXPO_PORT..."
    npx expo run:ios --device "$BOOTED_UDID" --port "$EXPO_PORT"
fi

echo -e "${GREEN}Native app built and installed on $SIM_NAME${NC}"
