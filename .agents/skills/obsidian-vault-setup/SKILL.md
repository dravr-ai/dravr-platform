---
name: obsidian-vault-setup
description: Use when setting up the shared dravr-vault Obsidian vault on a new machine or for
  a new team member. Guides through cloning, installing plugins, symlinking claude_docs, and
  verifying the Obsidian CLI and Work Log auto-filing.
user-invocable: true
metadata:
  version: "1.1.0"
  domain: devops
  triggers: obsidian, vault, setup, onboard, dravr-vault
  role: specialist
  scope: implementation
  output-format: report
---

# Obsidian Vault Setup

## Role

DevOps assistant for setting up the shared dravr-vault Obsidian knowledge base on a developer machine.

## When to Use

Invoke when a team member needs to:
- Clone and open the dravr-vault for the first time
- Connect their dravr-platform checkout to the vault via symlink
- Verify the Obsidian CLI, the commit timer and Templater's Work Log auto-filing

## Defaults

```
PLATFORM_DIR = the current dravr-platform checkout
VAULT_DIR    = $PLATFORM_DIR/../dravr-vault
VAULT_REPO   = https://github.com/dravr-ai/dravr-vault.git
```

The vault must sit next to dravr-platform: the SessionStart hook, `vault-sync.sh` and
`.claude/rules/shared-memory.md` all reach it as `../dravr-vault`. Clone over HTTPS —
SSH has no agent in tmux/SSH sessions, and `gh auth setup-git` supplies the credential.

## Workflow

### Step 1: Verify prerequisites

Check that the following are installed:

```bash
# Obsidian desktop app (required)
ls /Applications/Obsidian.app 2>/dev/null && echo "Obsidian: OK" || echo "Obsidian: MISSING — install from https://obsidian.md"

# The app's own CLI (Settings → General → enable the command line interface)
command -v obsidian   # must print /Applications/Obsidian.app/Contents/MacOS/obsidian
```

If `command -v obsidian` prints an npm path, the unrelated `obsidian-cli` npm package is
shadowing the real CLI — `npm uninstall -g obsidian-cli`.

### Step 2: Clone dravr-vault

```bash
# Only if not already present
if [ ! -d "$VAULT_DIR" ]; then
  git clone https://github.com/dravr-ai/dravr-vault.git "$VAULT_DIR"
fi

# Verify on main branch
cd "$VAULT_DIR" && git status
```

### Step 3: Install plugins

Downloads obsidian-git, Templater and Local REST API, and writes pre-configured settings.

```bash
cd "$VAULT_DIR"
./scripts/install-plugins.sh
```

This script:
- Downloads `main.js` + `manifest.json` for each plugin from GitHub releases
- Copies pre-configured `data.json` settings from `.obsidian/plugin-configs/`, including
  Templater's `Work Log` → `Templates/Work Log.md` folder template
- Writes `.obsidian/community-plugins.json` to enable all three plugins

Plugin binaries are excluded from git (`.obsidian/plugins/` is in `.gitignore`).
Re-running the script is safe — it overwrites existing files idempotently.

### Step 4: Create claude_docs symlink

This links `dravr-platform/claude_docs/` to `dravr-vault/Work Log/` so Claude Code
session outputs land directly in the vault.

```bash
cd "$PLATFORM_DIR"

# Safety check: do NOT overwrite a real directory
if [ -d claude_docs ] && [ ! -L claude_docs ]; then
  echo "ERROR: claude_docs/ exists as a real directory. Back it up before proceeding."
  exit 1
fi

ln -sfn "$VAULT_DIR/Work Log" claude_docs
ls -la claude_docs   # verify symlink resolves
```

### Step 5: Open vault in Obsidian

Instruct the user to:
1. Open Obsidian
2. Click **Open folder as vault**
3. Navigate to `$VAULT_DIR` and click **Open**
4. When prompted "Trust and enable plugins?", click **Trust author and enable plugins**

All three plugins are active immediately — no manual browsing required.

Pick one 10-minute commit timer per machine: obsidian-git's (the default config), or the
launchd `vault-sync` agent (`vault-sync install`), which also runs while Obsidian is closed
and posts a macOS notification on failure. With the launchd agent, set obsidian-git's
auto-commit and auto-push intervals to 0.

### Step 6: Verify the symlink and auto-filing

```bash
# The symlink resolves into the vault
ls "$PLATFORM_DIR/claude_docs/README.md"

# Templater files a note created at the Work Log root into this month's bucket
obsidian create path="Work Log/zz-setup-probe.md"
ls "$VAULT_DIR/Work Log/$(date +%Y-%m)/zz-setup-probe.md"   # must exist, with type: worklog frontmatter
obsidian delete path="Work Log/$(date +%Y-%m)/zz-setup-probe.md" permanent
```

If the probe stays at the `Work Log/` root, check that `Templates/Work Log.md` still opens
with its `<%* … tp.file.move … %>` block and that Settings → Templater → Folder Templates
maps `Work Log` → `Templates/Work Log.md`; reload Obsidian after fixing either.

## Constraints

- NEVER overwrite an existing `claude_docs/` directory that contains real files
- ALWAYS verify dravr-vault is on `main` branch before linking
- NEVER install the npm `obsidian-cli` package — it shadows the app's own `obsidian` CLI
- NEVER open `Templates/Work Log.md` in a way that lets Templater render it — that replaces the template with a note

## Success Criteria

- `ls -la $PLATFORM_DIR/claude_docs` shows a symlink pointing to `$VAULT_DIR/Work Log`
- Obsidian opens the vault and shows all folders (Architecture, APIs, Methodology, Development)
- `command -v obsidian` resolves to the app bundle
- Exactly one commit timer runs: obsidian-git's, or the launchd `vault-sync` agent
- The Step 6 probe lands in the current month's `Work Log/` bucket
