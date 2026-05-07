#!/usr/bin/env python3
"""
SDK Review Harness

Compares each Rust SDK module against its Go/Node.js/Python counterparts by
invoking the Claude Code CLI. No separate API key required — uses your existing
Claude Code session. Results are saved as markdown to review-output/YYYY-MM-DD/.
Progress is tracked in review-output/progress.json so runs can resume.

Usage:
    python scripts/sdk_review.py --module output
    python scripts/sdk_review.py --all
    python scripts/sdk_review.py --all --max 5
    python scripts/sdk_review.py --report
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys
from datetime import date
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).parent.parent
MODULES_FILE = REPO_ROOT / "scripts" / "sdk-review-modules.yml"
OUTPUT_DIR = REPO_ROOT / "review-output"
PROGRESS_FILE = OUTPUT_DIR / "progress.json"

_UPSTREAM_SENTINEL = REPO_ROOT / "upstream" / "sdk" / "go" / "pulumi" / "types.go"

# Per-file byte cap to avoid hitting context limits on very large upstream files.
MAX_FILE_BYTES = 80_000

_SYSTEM_PROMPT = (
    "You are a senior Rust engineer reviewing a native Rust Pulumi SDK. "
    "Your task is to compare a Rust module against its counterparts in Go, Node.js, and Python "
    "and identify gaps, missing features, behavioral divergences, and API surface differences. "
    "Be concrete: cite specific function names, types, and behaviors. "
    "Structure your review with these sections:\n\n"
    "## Summary\nOverall assessment in 2-3 sentences.\n\n"
    "## Missing Features\nFeatures present in upstream SDKs but absent from Rust.\n\n"
    "## Behavioral Divergences\nWhere the Rust implementation differs in semantics or behavior.\n\n"
    "## API Surface Gaps\nPublic API elements (types, methods, traits) missing from Rust.\n\n"
    "## Recommendations\nPrioritized list of what to implement first, with rationale.\n\n"
    "## Verdict\nOne of: COMPLETE, MINOR GAPS, SIGNIFICANT GAPS, or MAJOR GAPS."
)


def ensure_upstream() -> None:
    """Initialize the upstream submodule if it hasn't been checked out yet."""
    if _UPSTREAM_SENTINEL.exists():
        return
    print("upstream/ submodule not initialized — running git submodule update --init upstream/ ...")
    result = subprocess.run(
        ["git", "submodule", "update", "--init", "--depth", "1", "upstream"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(f"ERROR: failed to initialize upstream submodule:\n{result.stderr}", file=sys.stderr)
        sys.exit(1)
    print("upstream/ initialized.")


def ensure_claude_cli() -> None:
    """Verify the claude CLI is available."""
    result = subprocess.run(["claude", "--version"], capture_output=True)
    if result.returncode != 0:
        print("ERROR: 'claude' CLI not found. Make sure Claude Code is installed.", file=sys.stderr)
        sys.exit(1)


# ── File I/O ──────────────────────────────────────────────────────────────────

def read_file(path: Path, label: str) -> str:
    if not path.exists():
        return f"[FILE NOT FOUND: {path}]"
    content = path.read_text(errors="replace")
    if len(content.encode()) > MAX_FILE_BYTES:
        truncated = content.encode()[:MAX_FILE_BYTES].decode(errors="replace")
        content = truncated + f"\n\n... [truncated — showing first {MAX_FILE_BYTES} bytes of {label}]"
    return content


def build_upstream_text(module: dict) -> str:
    sections = []
    for lang_key, lang_label in [("go", "Go"), ("nodejs", "Node.js"), ("python", "Python")]:
        for rel_path in (module.get(lang_key) or []):
            content = read_file(REPO_ROOT / rel_path, rel_path)
            sections.append(f"### {lang_label}: {rel_path}\n\n```\n{content}\n```")
    return "\n\n".join(sections)


def build_prompt(module: dict) -> str:
    name = module["name"]
    description = module["description"]
    rust_content = read_file(REPO_ROOT / module["rust"], module["rust"])
    upstream_text = build_upstream_text(module)
    return (
        f"# Module: `{name}`\n\n"
        f"**Description:** {description}\n\n"
        f"## Upstream SDK Implementations\n\n"
        f"{upstream_text}\n\n"
        f"## Rust Implementation\n\n"
        f"### {module['rust']}\n\n"
        f"```rust\n{rust_content}\n```\n\n"
        f"Please review the Rust implementation above against the upstream SDKs "
        f"and identify all gaps, missing features, and behavioral differences."
    )


# ── Staleness tracking ────────────────────────────────────────────────────────

def _hash_file(path: Path) -> str:
    if not path.exists():
        return ""
    return hashlib.sha256(path.read_bytes()).hexdigest()[:16]


def _upstream_sha() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=REPO_ROOT / "upstream",
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()[:16] if result.returncode == 0 else ""


def compute_hashes(module: dict) -> dict:
    hashes: dict = {"upstream_sha": _upstream_sha()}
    hashes[module["rust"]] = _hash_file(REPO_ROOT / module["rust"])
    for lang_key in ("go", "nodejs", "python"):
        for rel_path in (module.get(lang_key) or []):
            hashes[rel_path] = _hash_file(REPO_ROOT / rel_path)
    return hashes


def is_stale(module: dict, progress: dict) -> bool:
    entry = progress.get(module["name"], {})
    if entry.get("status") != "done":
        return False
    stored = entry.get("hashes", {})
    if not stored:
        return True  # reviewed before hash tracking was added
    return compute_hashes(module) != stored


# ── Progress ──────────────────────────────────────────────────────────────────

def load_modules() -> list[dict]:
    with open(MODULES_FILE) as f:
        return yaml.safe_load(f)["modules"]


def load_progress() -> dict:
    if PROGRESS_FILE.exists():
        with open(PROGRESS_FILE) as f:
            return json.load(f)
    return {}


def save_progress(progress: dict) -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    with open(PROGRESS_FILE, "w") as f:
        json.dump(progress, f, indent=2)


def print_report(modules: list[dict], progress: dict) -> None:
    print(f"\nSDK Review Progress — {len(modules)} modules total\n")
    print(f"{'Module':<25} {'Status':<12} {'Date':<12} {'Verdict'}")
    print("-" * 70)
    for m in modules:
        name = m["name"]
        entry = progress.get(name, {})
        status = entry.get("status", "pending")
        if status == "done" and is_stale(m, progress):
            status = "stale"
        reviewed_date = entry.get("date", "")
        verdict = entry.get("verdict", "")
        print(f"{name:<25} {status:<12} {reviewed_date:<12} {verdict}")

    done = sum(1 for m in modules if progress.get(m["name"], {}).get("status") == "done")
    stale = sum(1 for m in modules if is_stale(m, progress))
    suffix = f", {stale} stale" if stale else ""
    print(f"\n{done}/{len(modules)} complete{suffix}")


# ── Review ────────────────────────────────────────────────────────────────────

def extract_verdict(text: str) -> str:
    for line in text.splitlines():
        if "## Verdict" in line:
            continue
        lower = line.lower()
        for v in ["major gaps", "significant gaps", "minor gaps", "complete"]:
            if v in lower:
                return v.upper()
    return "UNKNOWN"


def review_module(module: dict) -> str:
    """Invoke the Claude Code CLI to review one module. Returns markdown content."""
    prompt = build_prompt(module)

    result = subprocess.run(
        [
            "claude",
            "--print",
            "--output-format", "text",
            "--no-session-persistence",
            "--tools", "",
            "--system-prompt", _SYSTEM_PROMPT,
        ],
        input=prompt,
        capture_output=True,
        text=True,
        timeout=600,
        cwd=REPO_ROOT,
    )

    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or f"claude exited with code {result.returncode}")

    review_text = result.stdout.strip()
    header = (
        f"# SDK Review: `{module['name']}`\n\n"
        f"**Date:** {date.today().isoformat()}  \n"
        f"**Rust file:** `{module['rust']}`  \n\n"
        "---\n\n"
    )
    return header + review_text


def save_result(module_name: str, content: str) -> Path:
    today = date.today().isoformat()
    day_dir = OUTPUT_DIR / today
    day_dir.mkdir(parents=True, exist_ok=True)
    out_path = day_dir / f"{module_name}.md"
    out_path.write_text(content)
    return out_path


def run_review(module: dict, progress: dict) -> None:
    name = module["name"]
    print(f"  Reviewing {name}...", end=" ", flush=True)
    try:
        result = review_module(module)
        out_path = save_result(name, result)
        verdict = extract_verdict(result)
        progress[name] = {
            "status": "done",
            "date": date.today().isoformat(),
            "verdict": verdict,
            "output": str(out_path.relative_to(REPO_ROOT)),
            "hashes": compute_hashes(module),
        }
        save_progress(progress)
        print(f"done [{verdict}] → {out_path.relative_to(REPO_ROOT)}")
    except Exception as e:
        progress[name] = {"status": "error", "date": date.today().isoformat(), "error": str(e)}
        save_progress(progress)
        print(f"ERROR: {e}")


# ── CLI ───────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="Pulumi Rust SDK review harness")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--module", metavar="NAME", help="Review a single module by name")
    group.add_argument("--all", action="store_true", help="Review all pending and stale modules")
    group.add_argument("--report", action="store_true", help="Print progress report and exit")
    parser.add_argument("--max", type=int, metavar="N", default=None, help="Stop after N modules")
    parser.add_argument("--force", action="store_true", help="Re-review even if up-to-date")
    args = parser.parse_args()

    ensure_upstream()

    modules = load_modules()
    progress = load_progress()

    if args.report:
        print_report(modules, progress)
        return

    ensure_claude_cli()

    if args.module:
        target = next((m for m in modules if m["name"] == args.module), None)
        if target is None:
            names = [m["name"] for m in modules]
            print(f"ERROR: unknown module '{args.module}'. Known: {', '.join(names)}", file=sys.stderr)
            sys.exit(1)
        if not args.force and progress.get(args.module, {}).get("status") == "done" and not is_stale(target, progress):
            print(f"Module '{args.module}' is up-to-date. Use --force to re-review.")
            return
        run_review(target, progress)

    elif args.all:
        pending = [
            m for m in modules
            if args.force
            or progress.get(m["name"], {}).get("status") != "done"
            or is_stale(m, progress)
        ]
        if not pending:
            print("All modules up-to-date. Use --force to re-review.")
            return
        if args.max is not None:
            pending = pending[: args.max]
        print(f"Reviewing {len(pending)} module(s)...")
        for m in pending:
            run_review(m, progress)
        print("\nDone.")
        print_report(modules, progress)


if __name__ == "__main__":
    main()
