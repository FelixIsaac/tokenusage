"""Thin wrapper that downloads the prebuilt ``tu`` binary on first run."""

from __future__ import annotations

import io
import os
import platform
import stat
import subprocess
import sys
import tarfile
import urllib.request

from tokenusage import __version__

OWNER = "hanbu97"
REPO = "tokenusage"
VERSION_TAG = f"v{__version__}"


def _resolve_target() -> str | None:
    system = platform.system().lower()
    machine = platform.machine().lower()

    if system == "darwin" and machine in ("arm64", "aarch64"):
        return "aarch64-apple-darwin"
    if system == "darwin" and machine in ("x86_64", "amd64"):
        return "x86_64-apple-darwin"
    if system == "linux" and machine in ("x86_64", "amd64"):
        return "x86_64-unknown-linux-gnu"
    if system == "windows" and machine in ("x86_64", "amd64", "amd64"):
        return "x86_64-pc-windows-msvc"
    return None


def _cache_dir() -> str:
    """Platform-appropriate cache directory for the binary."""
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA", os.path.expanduser("~"))
        return os.path.join(base, "tokenusage", "bin")
    xdg = os.environ.get("XDG_CACHE_HOME", os.path.join(os.path.expanduser("~"), ".cache"))
    return os.path.join(xdg, "tokenusage", "bin")


def _bin_path() -> str:
    name = "tu.exe" if sys.platform == "win32" else "tu"
    return os.path.join(_cache_dir(), VERSION_TAG, name)


def _download(url: str) -> bytes:
    """Download with redirect following."""
    req = urllib.request.Request(url, headers={"User-Agent": "tokenusage-installer"})
    with urllib.request.urlopen(req, timeout=120) as resp:
        return resp.read()


def _ensure_binary() -> str:
    """Return the path to the ``tu`` binary, downloading if needed."""
    bin_path = _bin_path()
    if os.path.isfile(bin_path):
        return bin_path

    target = _resolve_target()
    if target is None:
        print(
            f"[tokenusage] Unsupported platform: {platform.system()}/{platform.machine()}",
            file=sys.stderr,
        )
        print(
            "[tokenusage] Install from source: cargo install tokenusage --bin tu",
            file=sys.stderr,
        )
        sys.exit(1)

    is_windows = sys.platform == "win32"

    if is_windows:
        asset_name = f"tu-{VERSION_TAG}-{target}.exe"
    else:
        asset_name = f"tu-{VERSION_TAG}-{target}.tar.gz"

    url = f"https://github.com/{OWNER}/{REPO}/releases/download/{VERSION_TAG}/{asset_name}"

    print(f"[tokenusage] Downloading {asset_name}...", file=sys.stderr)
    try:
        data = _download(url)
    except Exception as exc:
        print(f"[tokenusage] Download failed: {exc}", file=sys.stderr)
        print(
            "[tokenusage] Install from source: cargo install tokenusage --bin tu",
            file=sys.stderr,
        )
        sys.exit(1)

    os.makedirs(os.path.dirname(bin_path), exist_ok=True)

    if is_windows:
        with open(bin_path, "wb") as f:
            f.write(data)
    else:
        # Extract binary from tar.gz archive.
        inner_dir = f"tu-{VERSION_TAG}-{target}"
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tf:
            member = tf.getmember(f"{inner_dir}/tu")
            reader = tf.extractfile(member)
            if reader is None:
                print("[tokenusage] Failed to extract binary from archive", file=sys.stderr)
                sys.exit(1)
            with open(bin_path, "wb") as f:
                f.write(reader.read())
        os.chmod(bin_path, os.stat(bin_path).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    print(f"[tokenusage] Installed {asset_name}", file=sys.stderr)
    return bin_path


def main() -> None:
    bin_path = _ensure_binary()
    try:
        raise SystemExit(subprocess.call([bin_path] + sys.argv[1:]))
    except KeyboardInterrupt:
        sys.exit(130)
