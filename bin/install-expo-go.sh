#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Installs the Expo Go that matches the project's Expo SDK on the booted iOS Simulator
# ABOUTME: Fast path for development — no Xcode build and no Metro; works with Simulator.app and Device Hub

set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

EXPO_PORT="${EXPO_PORT:-8082}"

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

echo "Simulator: $(ios_sim_device_name "$BOOTED_UDID") ($BOOTED_UDID)"

# The install goes straight through simctl rather than through
# `expo start --ios`, which also opens the simulator app — the step Xcode 27
# broke — and asks before replacing an older Expo Go.
INSTALLED="$(ios_sim_expo_go_version "$BOOTED_UDID")"
if ios_sim_ensure_expo_go "$BOOTED_UDID" "$PROJECT_ROOT/frontend-mobile"; then
    NOW="$(ios_sim_expo_go_version "$BOOTED_UDID")"
    if [ "$INSTALLED" = "$NOW" ]; then
        echo -e "${GREEN}Expo Go $NOW is already installed${NC}"
    else
        echo -e "${GREEN}Expo Go $NOW installed${INSTALLED:+ (replaced $INSTALLED)}${NC}"
    fi
    exit 0
fi

echo -e "${YELLOW}Could not install Expo Go. Try running manually:${NC}"
echo "    cd frontend-mobile && npx expo start --ios --go --port $EXPO_PORT"
exit 1
