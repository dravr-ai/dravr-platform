#!/bin/sh
# ABOUTME: Wrapper script for headless Chromium in containers without X11/Wayland
# ABOUTME: Injects --ozone-platform=headless so chromiumoxide can launch Chrome in Cloud Run
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

exec /usr/lib/chromium/chromium \
    --ozone-platform=headless \
    --disable-software-rasterizer \
    "$@"
