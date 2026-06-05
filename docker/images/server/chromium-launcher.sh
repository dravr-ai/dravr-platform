#!/bin/sh
# ABOUTME: Wrapper that launches Chromium under a virtual X display (Xvfb) so it
# ABOUTME: runs headed in Cloud Run, where Garmin/Google SSO block headless Chrome.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# chromiumoxide invokes this binary via the CHROME env var. dravr-sciotte drives
# two launch modes through it: activity scraping passes --headless (ignored under
# Xvfb, costs nothing), while credential login (credential_login_headless=false)
# passes no --headless and needs a real display — Garmin's SSO serves a bot wall
# to true-headless Chrome, so the selector login times out. xvfb-run gives that
# headed launch a virtual framebuffer; -a auto-allocates a free server number so
# concurrent logins each get an isolated display.

exec xvfb-run -a --server-args='-screen 0 1280x1024x24' \
    /usr/lib/chromium/chromium \
    --disable-software-rasterizer \
    "$@"
