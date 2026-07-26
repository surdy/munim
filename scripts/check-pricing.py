#!/usr/bin/env python3
"""Detect drift between pricing.toml and published Claude rates.

Why this exists: a released model with no matching row in pricing.toml does not error —
it silently bills at the `claude_default` catch-all. Claude Fable 5 was under-reported by
~3.3x for weeks that way, and the only symptom was an "Unknown" slice in a chart. This
job turns that class of bug into a failing check.

Deliberately a *reporter*, not a fixer. It never edits pricing.toml: rates are a
judgment call (see EXPECTED_DIFFS), and a bot that rewrote them every week would fight
decisions a human already made.

Sources
-------
Anthropic publishes no machine-readable pricing feed. `GET /v1/models` is authoritative
for which models *exist* but carries no rates; the docs pages carry rates but omit cache
read/write, which is most of munim's token volume. So:

  * model list  — Anthropic `GET /v1/models` when ANTHROPIC_API_KEY is set (authoritative),
                  otherwise the first-party `claude-*` keys in the LiteLLM database.
  * rates       — LiteLLM's model_prices_and_context_window.json, the same source ccusage
                  uses. It carries input, output, cache-write and cache-read per model.

Scope: Claude only. munim's `[[codex]]` rows are match patterns rather than real model
ids, and there is no comparable authoritative list of what Codex writes into its logs, so
a Codex check would be guesswork. Left out rather than half-done.

Usage:  python3 scripts/check-pricing.py [--verbose]
Exit:   0 = no drift, 1 = drift found, 2 = could not reach a source.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PRICING_TOML = REPO / "pricing.toml"

LITELLM_URL = (
    "https://raw.githubusercontent.com/BerriAI/litellm/main/"
    "model_prices_and_context_window.json"
)
MODELS_API = "https://api.anthropic.com/v1/models?limit=100"

# Rates are per 1,000,000 tokens: (input, output, cache_write, cache_read).
Rate = tuple[float, float, float, float]

# Known, intentional disagreements with the reference.
#
# Pinned on *both* sides on purpose: if either munim's rate or the reference's rate
# moves, the waiver stops applying and the model is checked normally again. That way a
# temporary carve-out can't quietly outlive its reason — when Sonnet 5's intro period
# ends and the reference returns to $3/$15, this entry stops matching and the two agree
# on their own.
EXPECTED_DIFFS: dict[str, dict] = {
    "claude-sonnet-5": {
        "munim": (3.0, 15.0, 3.75, 0.30),
        "reference": (2.0, 10.0, 2.5, 0.20),
        "reason": (
            "Sonnet 5 has introductory pricing of $2/$10 per MTok through 2026-08-31. "
            "munim has no date-aware rates, so it bills the standard rate and slightly "
            "over-reports Sonnet 5 until then."
        ),
    },
}

# Retired models we deliberately no longer carry explicit rows for. They can still appear
# in old session logs, where the catch-all is an acceptable approximation.
IGNORE_MODELS = {
    "claude-2",
    "claude-2-1",
    "claude-instant-1",
    "claude-instant-1-2",
}


def fail(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(2)


def fetch_json(url: str, headers: dict[str, str] | None = None) -> dict:
    req = urllib.request.Request(url, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.load(r)
    except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError) as e:
        fail(f"could not fetch {url}: {e}")


# ─── munim's matcher, mirrored ───────────────────────────────────────────────
# Kept in sync with Pricing::find in crates/munim-core/src/pricing.rs: both sides are
# lowercased with `.`/`_` folded to `-`, and the first row whose key is a substring of
# the model id wins. Checking against a re-implementation rather than the config lets
# this catch ordering mistakes, not just missing rows.


def norm(s: str) -> str:
    return s.lower().replace(".", "-").replace("_", "-")


def load_pricing(path: Path = PRICING_TOML) -> dict:
    with path.open("rb") as f:
        return tomllib.load(f)


def resolve(pricing: dict, model: str) -> tuple[str | None, Rate]:
    """Return (matched row key, rate). A key of None means the catch-all was used."""
    n = norm(model)
    for row in pricing["claude"]:
        if norm(row["match"]) in n:
            return row["match"], (
                row["input"],
                row["output"],
                row.get("cache_write", 0.0),
                row["cache_read"],
            )
    d = pricing["claude_default"]
    return None, (d["input"], d["output"], d.get("cache_write", 0.0), d["cache_read"])


# ─── reference data ──────────────────────────────────────────────────────────


def litellm_rates() -> dict[str, Rate]:
    raw = fetch_json(LITELLM_URL)
    out: dict[str, Rate] = {}
    for key, v in raw.items():
        if not isinstance(v, dict) or v.get("litellm_provider") != "anthropic":
            continue
        if not key.startswith("claude-"):
            continue  # skip bedrock/vertex-prefixed aliases
        if v.get("input_cost_per_token") is None:
            continue
        out[key] = (
            v["input_cost_per_token"] * 1e6,
            v.get("output_cost_per_token", 0.0) * 1e6,
            v.get("cache_creation_input_token_cost", 0.0) * 1e6,
            v.get("cache_read_input_token_cost", 0.0) * 1e6,
        )
    return out


def anthropic_models() -> list[str] | None:
    """Authoritative model list, or None when no API key is configured."""
    key = os.environ.get("ANTHROPIC_API_KEY")
    if not key:
        return None
    data = fetch_json(
        MODELS_API, {"x-api-key": key, "anthropic-version": "2023-06-01"}
    )
    return [m["id"] for m in data.get("data", [])]


def fmt(r: Rate) -> str:
    return "/".join(f"{x:g}" for x in r)


def same(a: Rate, b: Rate) -> bool:
    """Reference rates are per-token values scaled by 1e6, so compare with a tolerance —
    exact equality spuriously fails on values like 2e-06 * 1e6."""
    return all(abs(x - y) <= 1e-9 for x, y in zip(a, b))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--verbose", action="store_true", help="list every model checked")
    ap.add_argument(
        "--pricing",
        type=Path,
        default=PRICING_TOML,
        help="pricing table to check (default: repo pricing.toml)",
    )
    args = ap.parse_args()

    pricing = load_pricing(args.pricing)
    reference = litellm_rates()
    live = anthropic_models()

    if live is not None:
        models = [m for m in live if m not in IGNORE_MODELS]
        print(f"model list: Anthropic /v1/models ({len(models)} models)")
    else:
        models = [m for m in reference if m not in IGNORE_MODELS]
        print(
            f"model list: LiteLLM ({len(models)} models) — set ANTHROPIC_API_KEY to use "
            "the authoritative /v1/models list instead"
        )
    print(f"rates:      LiteLLM ({len(reference)} Claude entries)\n")

    missing_row: list[str] = []
    drifted: list[tuple[str, str, Rate, Rate]] = []
    unverifiable: list[str] = []
    waived: list[str] = []

    for model in sorted(models):
        row, ours = resolve(pricing, model)
        theirs = reference.get(model)

        if row is None:
            missing_row.append(model)
            continue
        if theirs is None:
            unverifiable.append(model)
            continue

        exp = EXPECTED_DIFFS.get(model)
        if exp and same(tuple(exp["munim"]), ours) and same(tuple(exp["reference"]), theirs):
            waived.append(model)
            if args.verbose:
                print(f"  ~ {model:<32} waived ({fmt(ours)} vs {fmt(theirs)})")
            continue

        if not same(ours, theirs):
            drifted.append((model, row, ours, theirs))
        elif args.verbose:
            print(f"  . {model:<32} {row:<14} {fmt(ours)}")

    problems = 0

    if missing_row:
        problems += len(missing_row)
        print("MISSING ROW — these bill at the claude_default catch-all:")
        for m in missing_row:
            ref = reference.get(m)
            hint = f"  (reference says {fmt(ref)})" if ref else ""
            print(f"  {m}{hint}")
        print("  → add a [[claude]] row, above any less-specific row that also matches\n")

    if drifted:
        problems += len(drifted)
        print("RATE DRIFT — munim disagrees with the reference:")
        for m, row, ours, theirs in drifted:
            print(f"  {m}")
            print(f"    matched row : {row}")
            print(f"    munim       : {fmt(ours)}")
            print(f"    reference   : {fmt(theirs)}")
        print("  → update the row, or add an EXPECTED_DIFFS waiver with a reason\n")

    if unverifiable:
        print("NOT IN REFERENCE — rate could not be checked (not a failure):")
        for m in unverifiable:
            print(f"  {m}")
        print()

    for m in waived:
        print(f"waived: {m} — {EXPECTED_DIFFS[m]['reason']}")
    if waived:
        print()

    if problems:
        print(f"FAIL: {problems} pricing issue(s) found.")
        return 1

    print("OK: every current model has an explicit row and matches the reference.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
