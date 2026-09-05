# Maestro E2E Tests for Pierre Mobile

This directory contains Maestro E2E tests for the Pierre mobile app.

## Prerequisites

1. Install Maestro CLI:
   ```bash
   # macOS
   curl -Ls "https://get.maestro.mobile.dev" | bash

   # Linux
   curl -Ls "https://get.maestro.mobile.dev" | bash
   ```

2. Boot the stack with the app loaded in Expo Go on a booted simulator or emulator.
   The flows drive Expo Go (`host.exp.Exponent` on iOS, `host.exp.exponent` on Android)
   through the `exp://` deep link in `helpers/launch-app.yaml`, which branches on
   platform; `bun run ios` / `bun run android` are native Gradle/Xcode builds and are
   not what the flows expect.
   ```bash
   # iOS simulator (default)
   ../bin/setup-db-with-seeds-and-oauth-and-start-servers.sh

   # Android emulator — boot an AVD first, then:
   ANDROID_HOME=/opt/homebrew/share/android-commandlinetools \
     ../bin/setup-db-with-seeds-and-oauth-and-start-servers.sh --android
   ```

## Directory Structure

```
.maestro/
├── config.yaml              # Global configuration
├── helpers/                 # Reusable helper flows
│   ├── launch-app.yaml      # Launch app fresh
│   ├── login.yaml           # Perform login
│   ├── login-if-needed.yaml # Conditional login
│   ├── navigate-to-*.yaml   # Navigation helpers
│   ├── new-chat.yaml        # Start new conversation
│   ├── send-chat-message.yaml
│   ├── wait-for-response.yaml
│   ├── logout.yaml
│   └── go-back.yaml
├── login/                   # Login flow tests
├── settings/                # Settings screen tests
├── chat/                    # Chat functionality tests
├── store/                   # Discover/Store tests (browse, install → hint, uninstall)
├── discover/                # The edit sheet on an installed coach (the only coach editor)
├── voice-input/             # Voice input tests
├── synthetic-provider/      # Synthetic data provider tests
└── visual/                  # Visual regression tests
```

## Running Tests

```bash
# Run all tests
bun run maestro

# Run specific test suite
bun run maestro:login
bun run maestro:settings
bun run maestro:chat
# ... etc

# Run individual test file
maestro test .maestro/login/01-show-login-screen.yaml

# Run with CI output format
bun run maestro:ci
```

On Android, run flows **one file per `maestro test` invocation** — the way both CI
workflows do. Running the folder in one invocation restarts Maestro's on-device driver
during the first flow's `launchApp` and every later flow fails within milliseconds with
`Device server died … UNAVAILABLE` (observed 2026-09-05, Maestro 2.10.0, Pixel 6 API 33).

## Test Credentials

- Email: `mobiletest@pierre.dev`
- Password: `MobileTest1234`

These are configured in `config.yaml` as environment variables.

## Writing Tests

### Basic Test Structure

```yaml
appId: ai.dravr.app

---
# Import helpers at the start
- runFlow:
    file: ../helpers/launch-app.yaml
- runFlow:
    file: ../helpers/login.yaml

# Your test assertions
- assertVisible:
    id: "some-element"

- tapOn:
    id: "button-id"

- inputText: "Hello World"
```

### Common Patterns

| Detox | Maestro |
|-------|---------|
| `element(by.id('foo')).tap()` | `- tapOn: { id: "foo" }` |
| `element(by.text('Submit'))` | `- tapOn: "Submit"` |
| `element(by.id('input')).typeText('text')` | `- tapOn: { id: "input" }` then `- inputText: "text"` |
| `waitFor(element).toBeVisible()` | `- assertVisible: { id: "..." }` |
| `expect(element).not.toBeVisible()` | `- assertNotVisible: { id: "..." }` |
| `device.launchApp()` | `- launchApp` |
| `device.pressBack()` | `- back` |
| `element.clearText()` | `- clearText` |
| `element.longPress()` | `- longPressOn: { id: "..." }` |
| `element.scroll()` | `- scroll` or `- scrollUntilVisible` |

### Using Helper Flows

```yaml
# Run a helper flow
- runFlow:
    file: ../helpers/login.yaml

# Conditional flow execution
- runFlow:
    when:
      visible:
        id: "login-screen"
    file: ../helpers/login.yaml

# Pass environment variables
- runFlow:
    file: ../helpers/send-chat-message.yaml
    env:
      MESSAGE: "Hello Pierre"
```

## Environment Variables

Set in `config.yaml` or pass via command line:

```bash
maestro test --env TEST_EMAIL=custom@email.com .maestro/login/
```

## Debugging

```bash
# Run with verbose output
maestro test --debug .maestro/login/

# Take screenshot at failure
# (Enabled by default in config.yaml)

# Interactive mode (step through test)
maestro studio
```

## CI Integration

The tests are configured to run in CI with JUnit output:

```bash
bun run maestro:ci
```

This generates `maestro-results.xml` for CI reporting.
