#!/usr/bin/env python3
"""Cache-bust local asset references in deployable HTML files.

GitHub Pages doesn't let us set custom HTTP headers, so browser caching
relies entirely on what's in the HTML.  After a deploy, browsers can
serve stale cached `loft-gl.js` / `pkg/loft.js` / `loft_bg.wasm` for
~10 minutes (default GH Pages cache-control), forcing users to
hard-refresh every page individually.

This script:
1. Adds `<meta http-equiv="Cache-Control" content="no-cache, no-store,
   must-revalidate">` to the head of every TARGET HTML (so the HTML
   itself always fetches fresh — that's where new asset URLs live).
2. Rewrites each `<script src="X">`, `<link href="X">`, and ES module
   `import ... from 'X'` referencing a local JS / CSS / wasm file to
   include `?v=<md5-of-file>`.  When the asset's content changes the
   query changes, browsers see a new URL and fetch fresh.  When content
   is stable they keep the cached copy.

Targets only the HTML pages that load dynamic content — the prose
stdlib / keyword docs are left alone (they're static + benefit from
normal browser caching).

Re-run after `make gallery` / `make game`; safe to run on already-
busted HTML (the version-query is replaced, not appended twice).

Usage:
    python3 scripts/cache_bust_html.py
"""
import hashlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# HTML files that load dynamic JS / wasm.  Static prose docs are
# excluded — their normal caching is fine.
TARGETS = [
    "doc/gallery.html",
    "doc/gallery-run.html",
    "doc/playground.html",
    "doc/audience-demo/index.html",
    "doc/brick-buster.html",
]

# Cache-control meta tag inserted into <head>.
NO_CACHE_META = (
    '<meta http-equiv="Cache-Control" content="no-cache, no-store, must-revalidate">\n'
    '<meta http-equiv="Pragma" content="no-cache">\n'
    '<meta http-equiv="Expires" content="0">'
)
META_MARKER = "no-cache, no-store, must-revalidate"


def content_hash(path: Path) -> str:
    """First 12 hex chars of MD5 — short, stable, unique enough."""
    return hashlib.md5(path.read_bytes()).hexdigest()[:12]


def asset_hash_for(href: str, html_path: Path) -> str | None:
    """Resolve a relative href against html_path, return content hash
    if the target file exists, None if external / missing."""
    # External URLs pass through unchanged.
    if href.startswith(("http://", "https://", "//", "data:", "#")):
        return None
    # Already cache-busted — strip the existing ?v=... before resolving.
    href_clean = href.split("?", 1)[0]
    if not href_clean:
        return None
    target = (html_path.parent / href_clean).resolve()
    if not target.exists() or not target.is_file():
        return None
    # Only bust local assets; symlinks outside the repo are ignored.
    try:
        target.relative_to(ROOT)
    except ValueError:
        return None
    return content_hash(target)


def bust_url(match: re.Match, html_path: Path, prefix: str, suffix: str) -> str:
    """Helper used by re.sub for each script/link/import pattern."""
    href = match.group(1)
    h = asset_hash_for(href, html_path)
    if h is None:
        return match.group(0)
    href_clean = href.split("?", 1)[0]
    return f"{prefix}{href_clean}?v={h}{suffix}"


def process(html_path: Path) -> bool:
    """Rewrite one HTML.  Returns True if the file changed."""
    original = html_path.read_text()
    text = original

    # 1. Inject no-cache META if absent.  Place right after <head>.
    if META_MARKER not in text:
        text = re.sub(
            r"(<head[^>]*>)",
            r"\1\n  " + NO_CACHE_META.replace("\n", "\n  "),
            text,
            count=1,
        )

    # 2. Bust <script src="...">
    text = re.sub(
        r'<script([^>]*?)src="([^"]+)"',
        lambda m: f'<script{m.group(1)}src="' + (
            (m.group(2).split("?", 1)[0] + "?v=" + h) if (h := asset_hash_for(m.group(2), html_path)) else m.group(2)
        ) + '"',
        text,
    )

    # 3. Bust <link ... href="...">  (CSS, etc.)
    text = re.sub(
        r'<link([^>]*?)href="([^"]+)"',
        lambda m: f'<link{m.group(1)}href="' + (
            (m.group(2).split("?", 1)[0] + "?v=" + h) if (h := asset_hash_for(m.group(2), html_path)) else m.group(2)
        ) + '"',
        text,
    )

    # 4. Bust ES module imports — `import ... from 'X'` / `import ... from "X"`
    #    and bare `import('X')` dynamic imports.
    def bust_import(m: re.Match) -> str:
        prefix, open_q, href, close_q = m.group(1), m.group(2), m.group(3), m.group(4)
        h = asset_hash_for(href, html_path)
        if h is None:
            return m.group(0)
        href_clean = href.split("?", 1)[0]
        return f"{prefix}{open_q}{href_clean}?v={h}{close_q}"

    text = re.sub(
        r"(import\s+[^'\"]+\s+from\s+)(['\"])([^'\"]+)(['\"])",
        bust_import,
        text,
    )
    text = re.sub(
        r"(import\s*\(\s*)(['\"])([^'\"]+)(['\"])",
        bust_import,
        text,
    )

    if text != original:
        html_path.write_text(text)
        return True
    return False


def main():
    any_changed = False
    for rel in TARGETS:
        path = ROOT / rel
        if not path.exists():
            print(f"skip (missing): {rel}", file=sys.stderr)
            continue
        if process(path):
            print(f"rewrote: {rel}")
            any_changed = True
        else:
            print(f"unchanged: {rel}")
    return 0 if not any_changed else 0


if __name__ == "__main__":
    sys.exit(main())
