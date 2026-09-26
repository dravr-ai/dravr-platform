#!/usr/bin/env bash
# ABOUTME: iOS simulator helpers for the dev scripts: find or boot a device, show it, install and open Expo Go
# ABOUTME: Works with Simulator.app (Xcode 16 and 26) and with Device Hub (Xcode 27), which has no Simulator.app
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Xcode 27 replaced Simulator.app with Device Hub (com.apple.dt.Devices, at
# <Xcode>/Contents/Applications/DeviceHub.app). `open -a Simulator` fails
# there, and opening Device Hub does not boot a device, so a path that used to
# be "open the Simulator, then `expo start --ios`" has to spell the steps out:
# boot with simctl, show the device, install Expo Go, open the URL. @expo/cli
# 57 knows Device Hub as well, but nothing here depends on it, so the dev
# scripts do the same thing whichever Xcode is selected.
#
# Sourced, not executed. Every external tool is reached through PATH, which is
# how scripts/ci/ios-simulator-lib.test.sh runs this file against stubs.

IOS_SIM_SIMULATOR_APP_ID="com.apple.iphonesimulator"
IOS_SIM_DEVICE_HUB_APP_ID="com.apple.dt.Devices"
IOS_SIM_EXPO_GO_ID="host.exp.Exponent"
IOS_SIM_VERSIONS_URL="${IOS_SIM_VERSIONS_URL:-https://api.expo.dev/v2/versions/latest}"

# True when the selected Xcode still ships Simulator.app (Xcode 26 and older
# keep it under the developer dir; Xcode 27 does not have one).
ios_sim_has_simulator_app() {
    local developer_dir
    developer_dir="$(xcode-select -p 2>/dev/null)" || return 1
    [ -n "$developer_dir" ] && [ -d "$developer_dir/Applications/Simulator.app" ]
}

# The first booted iOS simulator's UDID, or nothing.
ios_sim_booted_udid() {
    xcrun simctl list devices booted -j 2>/dev/null | python3 -c '
import json, sys
for runtime, devices in json.load(sys.stdin).get("devices", {}).items():
    if ".iOS-" not in runtime:
        continue
    for device in devices:
        if device.get("state") == "Booted":
            print(device["udid"])
            sys.exit(0)
' 2>/dev/null || true
}

# A device's name ("iPhone 17 Pro"), or nothing.
ios_sim_device_name() { # udid
    xcrun simctl list devices -j 2>/dev/null | python3 -c '
import json, sys
for devices in json.load(sys.stdin).get("devices", {}).values():
    for device in devices:
        if device.get("udid") == sys.argv[1]:
            print(device.get("name", ""))
            sys.exit(0)
' "$1" 2>/dev/null || true
}

# The device to boot when none is: IOS_SIM_UDID when set, otherwise the first
# available iPhone on the newest installed iOS runtime.
ios_sim_pick_device() {
    if [ -n "${IOS_SIM_UDID:-}" ]; then
        echo "$IOS_SIM_UDID"
        return 0
    fi
    xcrun simctl list devices available -j 2>/dev/null | python3 -c '
import json, sys

def ios_version(runtime):
    tail = runtime.rsplit(".", 1)[-1]
    if not tail.startswith("iOS-"):
        return None
    return tuple(int(part) for part in tail[4:].split("-") if part.isdigit())

best = None
for runtime, devices in json.load(sys.stdin).get("devices", {}).items():
    version = ios_version(runtime)
    if version is None:
        continue
    for device in devices:
        if device.get("isAvailable", True) and device.get("name", "").startswith("iPhone"):
            if best is None or version > best[0]:
                best = (version, device["udid"])
            break
if best:
    print(best[1])
' 2>/dev/null || true
}

# Prints the UDID of a booted simulator, booting one first when none is. A
# pinned IOS_SIM_UDID wins over whatever else happens to be booted, so a second
# checkout never drives the device a peer's Maestro run is using.
ios_sim_ensure_booted() {
    local udid
    if [ -z "${IOS_SIM_UDID:-}" ]; then
        udid="$(ios_sim_booted_udid)"
        if [ -n "$udid" ]; then
            echo "$udid"
            return 0
        fi
    fi
    udid="$(ios_sim_pick_device)"
    if [ -z "$udid" ]; then
        echo "no available iPhone simulator; create one in Xcode or with xcrun simctl create" >&2
        return 1
    fi
    # -b boots the device unless it already is, then waits until it has.
    if ! xcrun simctl bootstatus "$udid" -b >/dev/null 2>&1; then
        echo "simulator $udid did not finish booting" >&2
        return 1
    fi
    echo "$udid"
}

# Brings the device's window forward. Best effort: a headless run has nothing
# to show, and the device is usable without a window.
ios_sim_show() { # udid
    if ios_sim_has_simulator_app; then
        open -b "$IOS_SIM_SIMULATOR_APP_ID" --args -CurrentDeviceUDID "$1" >/dev/null 2>&1 || true
    else
        # Device Hub registers devices:// and focuses the device named by it.
        open "devices://device/open?id=$1" >/dev/null 2>&1 ||
            open -b "$IOS_SIM_DEVICE_HUB_APP_ID" >/dev/null 2>&1 || true
    fi
}

# What to tell someone who has no simulator running. Device Hub does not boot a
# device on its own, so the boot comes first on every Xcode.
ios_sim_boot_hint() {
    echo "    xcrun simctl list devices available        # pick an iPhone"
    echo "    xcrun simctl boot \"<device name or UDID>\""
    echo "    open -b $IOS_SIM_SIMULATOR_APP_ID || open -b $IOS_SIM_DEVICE_HUB_APP_ID"
}

# Major version of the expo package the project resolves ("57"): the SDK the
# Expo Go build has to match.
ios_sim_project_sdk_major() { # project_dir
    (cd "$1" && node -p "require('expo/package.json').version.split('.')[0]") 2>/dev/null
}

# CFBundleShortVersionString of the Expo Go installed on a device, or nothing.
ios_sim_expo_go_version() { # udid
    local app
    app="$(xcrun simctl get_app_container "$1" "$IOS_SIM_EXPO_GO_ID" app 2>/dev/null)" || return 0
    [ -f "$app/Info.plist" ] || return 0
    python3 -c '
import plistlib, sys
with open(sys.argv[1], "rb") as handle:
    print(plistlib.load(handle).get("CFBundleShortVersionString", ""))
' "$app/Info.plist" 2>/dev/null || true
}

# The simulator build of Expo Go that Expo publishes for an SDK major.
ios_sim_expo_go_url() { # sdk_major
    curl -fsSL "$IOS_SIM_VERSIONS_URL" 2>/dev/null | python3 -c '
import json, sys
entry = json.load(sys.stdin).get("data", {}).get("sdkVersions", {}).get(sys.argv[1] + ".0.0") or {}
print(entry.get("iosClientUrl", ""))
' "$1" 2>/dev/null || true
}

# Installs the Expo Go that matches the project's SDK, unless it already is.
# An older Expo Go is replaced rather than kept: `expo start` would stop to ask
# "Install the recommended Expo Go version?", which a spawned, non-interactive
# process cannot answer. The download lands in @expo/cli's own cache
# (~/.expo/ios-simulator-app-cache/<name>.tar.app), so either path reuses it.
ios_sim_ensure_expo_go() { # udid project_dir
    local udid="$1" sdk installed url cache app tmp
    sdk="$(ios_sim_project_sdk_major "$2")"
    if [ -z "$sdk" ]; then
        echo "cannot read the expo version under $2; run bun install first" >&2
        return 1
    fi
    installed="$(ios_sim_expo_go_version "$udid")"
    if [ -n "$installed" ] && [ "${installed%%.*}" = "$sdk" ]; then
        return 0
    fi
    url="$(ios_sim_expo_go_url "$sdk")"
    if [ -z "$url" ]; then
        echo "Expo publishes no Expo Go simulator build for SDK $sdk ($IOS_SIM_VERSIONS_URL)" >&2
        return 1
    fi
    cache="${EXPO_HOME:-$HOME/.expo}/ios-simulator-app-cache"
    app="$cache/$(basename "$url" .gz).app"
    if [ ! -f "$app/Info.plist" ]; then
        tmp="$(mktemp -d)"
        if ! curl -fsSL -o "$tmp/expo-go.tar.gz" "$url" ||
            ! mkdir -p "$tmp/app" ||
            ! tar -xzf "$tmp/expo-go.tar.gz" -C "$tmp/app" ||
            [ ! -f "$tmp/app/Info.plist" ]; then
            rm -rf "$tmp"
            echo "could not download Expo Go from $url" >&2
            return 1
        fi
        mkdir -p "$cache"
        rm -rf "$app"
        mv "$tmp/app" "$app"
        rm -rf "$tmp"
    fi
    xcrun simctl install "$udid" "$app"
}

# True once Metro on this port answers its status probe.
ios_sim_wait_for_metro() { # port [timeout_secs]
    local port="$1" timeout="${2:-120}" waited=0
    while [ "$waited" -lt "$timeout" ]; do
        if [ "$(curl -fsS "http://127.0.0.1:$port/status" 2>/dev/null)" = "packager-status:running" ]; then
            return 0
        fi
        sleep 1
        waited=$((waited + 1))
    done
    return 1
}

# Opens a Metro URL in Expo Go. Expo Go is launched first, which on iOS 18
# makes openurl hand the URL straight to it (mobile-e2e-ios.yml relies on
# that). iOS 27 shows "Open in "Expo Go"?" on every openurl regardless, and
# Expo Go 57 reads no launch-argument URL that would get around it, so on
# iOS 27 the URL waits behind that sheet for one tap on Open.
ios_sim_open_in_expo_go() { # udid url
    xcrun simctl launch "$1" "$IOS_SIM_EXPO_GO_ID" >/dev/null 2>&1 || true
    sleep "${IOS_SIM_LAUNCH_SETTLE_SECS:-3}"
    xcrun simctl openurl "$1" "$2"
}
