#!/usr/bin/env bash
# ABOUTME: Fixture test for bin/ios-simulator.sh — the dev scripts' simulator helpers on Xcode 26 and Xcode 27
# ABOUTME: Pins device choice, the Simulator.app / Device Hub split, and the Expo Go install that must match the SDK
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Xcode 27 removed Simulator.app, so `open -a Simulator` and `expo start --ios`
# stopped being a way to get a device on screen, and the setup script now boots
# the device, installs Expo Go and opens the URL itself. Every one of those
# steps talks to xcrun, open, curl or the network, so every case here runs
# against PATH stubs that record what they were asked to do: nothing below
# touches a real simulator, and it runs the same on the Linux fast-gate runner.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/../../bin/ios-simulator.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() {
    echo "  ❌ $1"
    failures=$((failures + 1))
}

expect() { # label actual expected
    if [ "$2" = "$3" ]; then pass "$1"; else
        fail "$1"
        echo "      got:      $2"
        echo "      expected: $3"
    fi
}

expect_contains() { # label haystack needle
    if grep -qF -- "$3" <<<"$2"; then pass "$1"; else
        fail "$1"
        echo "      missing:  $3"
        echo "      in:       $2"
    fi
}

echo "ios-simulator.sh fixture tests"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
STUBS="$WORK/stubs"
FIX="$WORK/fixtures"
CALLS="$WORK/calls.log"
mkdir -p "$STUBS" "$FIX"

# --- stubs -------------------------------------------------------------------
# Each stub appends one line per call to $CALLS and answers from $FIX.

cat >"$STUBS/xcode-select" <<'EOF'
#!/usr/bin/env bash
[ -n "${STUB_DEVELOPER_DIR:-}" ] || exit 2
echo "$STUB_DEVELOPER_DIR"
EOF

cat >"$STUBS/xcrun" <<'EOF'
#!/usr/bin/env bash
echo "xcrun $*" >>"$CALLS"
[ "$1" = simctl ] || exit 64
shift
case "$1 ${2:-} ${3:-}" in
    "list devices booted") cat "$FIX/booted.json" ;;
    "list devices available") cat "$FIX/available.json" ;;
    "list devices -j") cat "$FIX/all.json" ;;
    bootstatus*) exit "${STUB_BOOTSTATUS_RC:-0}" ;;
    get_app_container*)
        [ -f "$FIX/installed/Info.plist" ] || exit 2
        echo "$FIX/installed" ;;
    install*)
        mkdir -p "$FIX/installed"
        cp "$3/Info.plist" "$FIX/installed/Info.plist" ;;
    launch* | openurl*) exit 0 ;;
    *) exit 64 ;;
esac
EOF

cat >"$STUBS/open" <<'EOF'
#!/usr/bin/env bash
echo "open $*" >>"$CALLS"
EOF

cat >"$STUBS/curl" <<'EOF'
#!/usr/bin/env bash
echo "curl $*" >>"$CALLS"
out="" url=""
while [ $# -gt 0 ]; do
    case "$1" in
        -o) out="$2"; shift 2 ;;
        -*) shift ;;
        *) url="$1"; shift ;;
    esac
done
case "$url" in
    */status) [ "${STUB_METRO_UP:-0}" = 1 ] || exit 7; printf 'packager-status:running' ;;
    */versions/*) cat "$FIX/versions.json" ;;
    *.tar.gz) cp "$FIX/expo-go.tar.gz" "$out" ;;
    *) exit 22 ;;
esac
EOF

cat >"$STUBS/sleep" <<'EOF'
#!/usr/bin/env bash
echo "sleep $*" >>"$CALLS"
EOF
chmod +x "$STUBS"/*

export PATH="$STUBS:$PATH" CALLS FIX
export EXPO_HOME="$WORK/expo-home"

# --- fixtures ----------------------------------------------------------------

UDID_IPHONE_27="22222222-0000-0000-0000-000000000027"
UDID_IPHONE_26="33333333-0000-0000-0000-000000000026"
UDID_WATCH="44444444-0000-0000-0000-000000000011"
UDID_PINNED="55555555-0000-0000-0000-000000000027"

# The iOS 27 runtime lists an iPad and an unavailable iPhone ahead of the
# iPhone the pick should land on.
devices_json() { # booted_udid (or "") -> a simctl `list devices -j` document
    python3 - "$1" <<'PY'
import json, sys
booted = sys.argv[1]
def dev(udid, name, available=True):
    return {"udid": udid, "name": name, "isAvailable": available,
            "state": "Booted" if udid == booted else "Shutdown"}
print(json.dumps({"devices": {
    "com.apple.CoreSimulator.SimRuntime.watchOS-11-0": [dev("44444444-0000-0000-0000-000000000011", "Apple Watch Series 10")],
    "com.apple.CoreSimulator.SimRuntime.iOS-26-4": [dev("33333333-0000-0000-0000-000000000026", "iPhone 16")],
    "com.apple.CoreSimulator.SimRuntime.iOS-27-0": [
        dev("11111111-0000-0000-0000-000000000027", "iPad (A16)"),
        dev("99999999-0000-0000-0000-000000000027", "iPhone 17e", available=False),
        dev("22222222-0000-0000-0000-000000000027", "iPhone 17"),
        dev("55555555-0000-0000-0000-000000000027", "iPhone 18 Pro"),
    ],
}}))
PY
}

set_booted() { # udid (or "")
    devices_json "$1" >"$FIX/all.json"
    python3 - "$FIX/all.json" >"$FIX/booted.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
print(json.dumps({"devices": {rt: [d for d in ds if d["state"] == "Booted"] for rt, ds in data["devices"].items()}}))
PY
    python3 - "$FIX/all.json" >"$FIX/available.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
print(json.dumps({"devices": {rt: [d for d in ds if d["isAvailable"]] for rt, ds in data["devices"].items()}}))
PY
}

write_plist() { # path version
    mkdir -p "$(dirname "$1")"
    python3 - "$1" "$2" <<'PY'
import plistlib, sys
with open(sys.argv[1], "wb") as handle:
    plistlib.dump({"CFBundleIdentifier": "host.exp.Exponent", "CFBundleShortVersionString": sys.argv[2]}, handle)
PY
}

cat >"$FIX/versions.json" <<'EOF'
{"data": {"sdkVersions": {
  "55.0.0": {"iosClientUrl": "https://github.com/expo/expo-go-releases/releases/download/Expo-Go-55.0.34/Expo-Go-55.0.34.tar.gz"},
  "57.0.0": {"iosClientUrl": "https://github.com/expo/expo-go-releases/releases/download/Expo-Go-57.0.9/Expo-Go-57.0.9.tar.gz"}
}}}
EOF

# The published archive holds the .app's contents at its root.
write_plist "$WORK/archive/Info.plist" "57.0.9"
tar -czf "$FIX/expo-go.tar.gz" -C "$WORK/archive" .

PROJECT="$WORK/project"
mkdir -p "$PROJECT/node_modules/expo"
echo '{"name": "expo", "version": "57.0.25"}' >"$PROJECT/node_modules/expo/package.json"

XCODE26="$WORK/Xcode-26.app/Contents/Developer"
XCODE27="$WORK/Xcode-27.app/Contents/Developer"
mkdir -p "$XCODE26/Applications/Simulator.app" "$XCODE27" "$WORK/Xcode-27.app/Contents/Applications/DeviceHub.app"

# shellcheck source=../../bin/ios-simulator.sh
. "$UNDER_TEST"

reset_calls() { : >"$CALLS"; }

# --- 1-3: which simulator app the selected Xcode ships ------------------------
if STUB_DEVELOPER_DIR="$XCODE26" ios_sim_has_simulator_app; then
    pass "Xcode 26 layout has Simulator.app"
else fail "Xcode 26 layout has Simulator.app"; fi
if STUB_DEVELOPER_DIR="$XCODE27" ios_sim_has_simulator_app; then
    fail "Xcode 27 layout (Device Hub only) has no Simulator.app"
else pass "Xcode 27 layout (Device Hub only) has no Simulator.app"; fi
if STUB_DEVELOPER_DIR="" ios_sim_has_simulator_app; then
    fail "no selected Xcode reads as no Simulator.app"
else pass "no selected Xcode reads as no Simulator.app"; fi

# --- 4-5: the booted device is an iOS one --------------------------------------
set_booted "$UDID_WATCH"
expect "a booted watch is not an iOS simulator" "$(ios_sim_booted_udid)" ""
set_booted "$UDID_IPHONE_26"
expect "the booted iPhone is found" "$(ios_sim_booted_udid)" "$UDID_IPHONE_26"

# --- 6-7: the device to boot ---------------------------------------------------
set_booted ""
expect "picks the first available iPhone on the newest iOS runtime" "$(ios_sim_pick_device)" "$UDID_IPHONE_27"
expect "IOS_SIM_UDID pins the device" "$(IOS_SIM_UDID="$UDID_PINNED" ios_sim_pick_device)" "$UDID_PINNED"

# --- 8-12: booting ---------------------------------------------------------------
set_booted "$UDID_IPHONE_26"
reset_calls
expect "an already booted device is reused" "$(ios_sim_ensure_booted)" "$UDID_IPHONE_26"
if grep -q bootstatus "$CALLS"; then fail "reusing a booted device boots nothing"; else
    pass "reusing a booted device boots nothing"
fi

set_booted ""
reset_calls
expect "with none booted, the picked device is booted" "$(ios_sim_ensure_booted)" "$UDID_IPHONE_27"
expect_contains "the boot waits for the device" "$(cat "$CALLS")" "xcrun simctl bootstatus $UDID_IPHONE_27 -b"

set_booted "$UDID_IPHONE_26"
reset_calls
expect "a pinned device wins over another checkout's booted one" \
    "$(IOS_SIM_UDID="$UDID_PINNED" ios_sim_ensure_booted)" "$UDID_PINNED"

set_booted ""
if STUB_BOOTSTATUS_RC=1 ios_sim_ensure_booted >/dev/null 2>&1; then
    fail "a device that never finishes booting is an error"
else pass "a device that never finishes booting is an error"; fi

# --- 13-15: showing the device --------------------------------------------------
reset_calls
STUB_DEVELOPER_DIR="$XCODE26" ios_sim_show "$UDID_IPHONE_27"
expect "Xcode 26 shows the device in Simulator.app" "$(cat "$CALLS")" \
    "open -b com.apple.iphonesimulator --args -CurrentDeviceUDID $UDID_IPHONE_27"
reset_calls
STUB_DEVELOPER_DIR="$XCODE27" ios_sim_show "$UDID_IPHONE_27"
expect "Xcode 27 shows the device through Device Hub's URL scheme" "$(cat "$CALLS")" \
    "open devices://device/open?id=$UDID_IPHONE_27"
hint="$(ios_sim_boot_hint)"
expect_contains "the hint boots first, then opens either app" "$hint" \
    "open -b com.apple.iphonesimulator || open -b com.apple.dt.Devices"

# --- 16-18: which Expo Go the project needs -------------------------------------
expect "the SDK major comes from the resolved expo package" "$(ios_sim_project_sdk_major "$PROJECT")" "57"
expect "the SDK 57 Expo Go build is resolved" "$(ios_sim_expo_go_url 57)" \
    "https://github.com/expo/expo-go-releases/releases/download/Expo-Go-57.0.9/Expo-Go-57.0.9.tar.gz"
expect "an SDK Expo publishes no build for resolves to nothing" "$(ios_sim_expo_go_url 99)" ""

# --- 19-23: installing Expo Go -----------------------------------------------------
rm -rf "$FIX/installed"
reset_calls
if ios_sim_ensure_expo_go "$UDID_IPHONE_27" "$PROJECT" >/dev/null 2>&1; then
    pass "a missing Expo Go is installed"
else fail "a missing Expo Go is installed"; fi
CACHED_APP="$EXPO_HOME/ios-simulator-app-cache/Expo-Go-57.0.9.tar.app"
expect_contains "it goes through @expo/cli's own cache path" "$(cat "$CALLS")" \
    "xcrun simctl install $UDID_IPHONE_27 $CACHED_APP"
expect "the installed version is read back" "$(ios_sim_expo_go_version "$UDID_IPHONE_27")" "57.0.9"

reset_calls
ios_sim_ensure_expo_go "$UDID_IPHONE_27" "$PROJECT" >/dev/null 2>&1 || true
if grep -qE "simctl install|\.tar\.gz" "$CALLS"; then
    fail "a matching Expo Go is left alone"
else pass "a matching Expo Go is left alone"; fi

write_plist "$FIX/installed/Info.plist" "55.0.34"
reset_calls
ios_sim_ensure_expo_go "$UDID_IPHONE_27" "$PROJECT" >/dev/null 2>&1 || true
calls="$(cat "$CALLS")"
if grep -q "simctl install" <<<"$calls" && ! grep -q "\.tar\.gz" <<<"$calls"; then
    pass "an SDK 55 Expo Go is replaced from the cache without a second download"
else
    fail "an SDK 55 Expo Go is replaced from the cache without a second download"
    echo "$calls" | sed 's/^/      /'
fi

echo '{"name": "expo", "version": "99.0.0"}' >"$PROJECT/node_modules/expo/package.json"
rm -rf "$FIX/installed"
if ios_sim_ensure_expo_go "$UDID_IPHONE_27" "$PROJECT" >/dev/null 2>&1; then
    fail "an SDK with no published Expo Go is an error, not a silent success"
else pass "an SDK with no published Expo Go is an error, not a silent success"; fi
echo '{"name": "expo", "version": "57.0.25"}' >"$PROJECT/node_modules/expo/package.json"

# --- 24-26: Metro and the URL ---------------------------------------------------------
if STUB_METRO_UP=1 ios_sim_wait_for_metro 8095 3; then pass "a running Metro is seen"; else
    fail "a running Metro is seen"
fi
if STUB_METRO_UP=0 ios_sim_wait_for_metro 8095 2; then
    fail "a Metro that never answers times out"
else pass "a Metro that never answers times out"; fi

reset_calls
ios_sim_open_in_expo_go "$UDID_IPHONE_27" "exp://127.0.0.1:8095"
expect "Expo Go is launched before the URL is opened" \
    "$(grep -E 'simctl (launch|openurl)' "$CALLS" | tr '\n' '|')" \
    "xcrun simctl launch $UDID_IPHONE_27 host.exp.Exponent|xcrun simctl openurl $UDID_IPHONE_27 exp://127.0.0.1:8095|"

echo ""
if [ "$failures" -ne 0 ]; then
    echo "❌ $failures ios-simulator case(s) failed"
    exit 1
fi
echo "✅ all ios-simulator cases passed"
