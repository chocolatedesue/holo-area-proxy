"""Shared lab path resolution for YQH-157 profiling tools.

Environment (CLI flags on entry scripts override these):
  YQH157_LAB / YQH157_WD  Lab root (configs, generated, state, evidence)
  EXPCTL                  Path to expctl binary
  EXPCTL_STATE_ROOT       expctl state root (default: $YQH157_LAB/state)
  HOLO_IMAGE              OCI image for holod nodes (default below)

Independent Stage B repro (YQH-184) must set YQH157_LAB to a *new* lab such as
/home/cnic/work/yqh184-profiling-repro — do not default-apply into the YQH-157
truth lab. Script fallback below keeps legacy path only when env is unset.
"""
from __future__ import annotations

import os
from pathlib import Path

_DEFAULT_LAB = Path("/home/cnic/work/yqh157-real-profiling")
_DEFAULT_EXPCTL = Path("/home/cnic/work/smu/build/linux/arm64/release/expctl")
_DEFAULT_HOLO_IMAGE = "docker.io/library/holo-bundle:yqh135-ee60831"


def lab_root(override: str | Path | None = None) -> Path:
    if override:
        return Path(override).expanduser().resolve()
    env = os.environ.get("YQH157_LAB") or os.environ.get("YQH157_WD")
    return Path(env).expanduser().resolve() if env else _DEFAULT_LAB


def expctl_bin(override: str | Path | None = None) -> Path:
    if override:
        return Path(override).expanduser().resolve()
    env = os.environ.get("EXPCTL")
    return Path(env).expanduser().resolve() if env else _DEFAULT_EXPCTL


def state_root(lab: Path | None = None, override: str | Path | None = None) -> Path:
    if override:
        return Path(override).expanduser().resolve()
    env = os.environ.get("EXPCTL_STATE_ROOT")
    if env:
        return Path(env).expanduser().resolve()
    return (lab or lab_root()) / "state"


def evidence_root(lab: Path | None = None, override: str | Path | None = None) -> Path:
    if override:
        return Path(override).expanduser().resolve()
    return (lab or lab_root()) / "evidence"


def holo_image(override: str | None = None) -> str:
    """Resolve holod OCI image; always prefer full docker.io/library/ form when possible."""
    if override:
        return override.strip()
    env = (os.environ.get("HOLO_IMAGE") or "").strip()
    if env:
        return env
    return _DEFAULT_HOLO_IMAGE
