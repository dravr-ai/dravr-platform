#!/usr/bin/env python3
# ABOUTME: Push platform prompt files to dravr-contremaitre, keeping manifest.json sha256 in sync
# ABOUTME: Only PUTs files whose content differs; preserves commit history on the contremaitre repo

"""Mirror `crates/pierre-llm/src/prompts/*.md` into dravr-contremaitre.

Invoked by `.github/workflows/contremaitre-sync.yml` on every merge to main
that touches a prompt file. For each file in the platform repo:

1. Compute local SHA-256.
2. Fetch the corresponding file from dravr-contremaitre.
3. If the SHA-256 differs (or FORCE=true), PUT the new content via the
   GitHub Contents API and record the new SHA-256.
4. After all files are synced, fetch manifest.json, update every changed
   sha256 entry, and PUT the new manifest. The dravr-platform commit SHA is
   embedded in the commit message for traceability.

Environment:
  GH_TOKEN              — PAT with write access to dravr-ai/dravr-contremaitre
  CONTREMAITRE_REPO     — e.g. "dravr-ai/dravr-contremaitre"
  CONTREMAITRE_BRANCH   — e.g. "main"
  FORCE                 — "true" to re-push every file regardless of sha
  COMMIT_SHA            — dravr-platform commit SHA for commit-message context
"""

import base64
import hashlib
import json
import os
import sys
from pathlib import Path

import requests

REPO_ROOT = Path(__file__).resolve().parents[2]
PROMPT_DIR = REPO_ROOT / "crates" / "pierre-llm" / "src" / "prompts"
MANIFEST_PATH = "manifest.json"

# Maps local filename → manifest key under `prompts.system` in manifest.json.
PROMPT_MAP = {
    "pierre_system.md": "pierre_system",
    "coach_generation.md": "coach_generation",
    "insight_validation.md": "insight_validation",
    "insight_generation.md": "insight_generation",
    "messaging_context.md": "messaging_context",
    "recommendation_analysis.md": "recommendation_analysis",
    "recommendation_system.md": "recommendation_system",
    "activity_analysis.md": "activity_analysis",
    "activity_analysis_system.md": "activity_analysis_system",
    "tool_discipline.md": "tool_discipline",
    "tool_discipline_messaging.md": "tool_discipline_messaging",
    "memory_extraction.md": "memory_extraction",
}


def api_url(repo: str, path: str) -> str:
    return f"https://api.github.com/repos/{repo}/contents/{path}"


def get_file(session: requests.Session, repo: str, path: str, branch: str) -> tuple[str | None, str | None]:
    """Return (remote_content_sha256, remote_file_sha) or (None, None) if missing."""
    resp = session.get(api_url(repo, path), params={"ref": branch}, timeout=30)
    if resp.status_code == 404:
        return None, None
    resp.raise_for_status()
    payload = resp.json()
    raw = base64.b64decode(payload["content"])
    return hashlib.sha256(raw).hexdigest(), payload["sha"]


def put_file(
    session: requests.Session,
    repo: str,
    path: str,
    branch: str,
    content_bytes: bytes,
    message: str,
    file_sha: str | None,
) -> None:
    body = {
        "message": message,
        "branch": branch,
        "content": base64.b64encode(content_bytes).decode("ascii"),
    }
    if file_sha is not None:
        body["sha"] = file_sha
    resp = session.put(api_url(repo, path), json=body, timeout=30)
    if not resp.ok:
        raise SystemExit(f"PUT {path} failed: {resp.status_code} {resp.text}")


def main() -> int:
    token = os.environ["GH_TOKEN"]
    repo = os.environ["CONTREMAITRE_REPO"]
    branch = os.environ["CONTREMAITRE_BRANCH"]
    force = os.environ.get("FORCE", "").lower() == "true"
    platform_sha = os.environ.get("COMMIT_SHA", "unknown")[:12]

    session = requests.Session()
    session.headers.update(
        {
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        }
    )

    changed: list[tuple[str, str]] = []  # (manifest_key, new_sha256)

    for filename, key in PROMPT_MAP.items():
        local_path = PROMPT_DIR / filename
        if not local_path.exists():
            print(f"  skip {filename}: not present in platform repo")
            continue
        local_bytes = local_path.read_bytes()
        local_sha = hashlib.sha256(local_bytes).hexdigest()

        remote_path = f"prompts/system/{filename}"
        remote_sha, file_sha = get_file(session, repo, remote_path, branch)

        if not force and remote_sha == local_sha:
            print(f"  ok   {filename} unchanged ({local_sha[:12]})")
            continue

        msg = f"sync {filename} from dravr-platform@{platform_sha}"
        put_file(session, repo, remote_path, branch, local_bytes, msg, file_sha)
        changed.append((key, local_sha))
        print(f"  put  {filename} → {local_sha[:12]}")

    if not changed:
        print("All prompts already in sync. Nothing to commit.")
        return 0

    # Fetch, mutate, and push manifest.json
    manifest_url = api_url(repo, MANIFEST_PATH)
    resp = session.get(manifest_url, params={"ref": branch}, timeout=30)
    resp.raise_for_status()
    payload = resp.json()
    manifest = json.loads(base64.b64decode(payload["content"]))
    manifest_file_sha = payload["sha"]

    system_entries = manifest["prompts"]["system"]
    for key, new_sha in changed:
        entry = system_entries.setdefault(key, {})
        entry["path"] = f"prompts/system/{key}.md"
        entry["sha256"] = new_sha

    manifest_bytes = (json.dumps(manifest, indent=2) + "\n").encode("utf-8")
    msg = f"sync manifest sha256 from dravr-platform@{platform_sha}"
    put_file(session, repo, MANIFEST_PATH, branch, manifest_bytes, msg, manifest_file_sha)
    print(f"  put  manifest.json (updated {len(changed)} entries)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
