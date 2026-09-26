"""Repository-root discovery for the archived Phase 5A research scripts.

This research directory is a task directory, so it moves when the task is
archived and its depth changes. Fixed ``parents[N]`` arithmetic therefore points
at ``.trellis/tasks`` instead of the repository. Derivations that need the
working tree resolve it from explicit repository markers instead.
"""
from pathlib import Path

MARKERS = ('go.mod', 'rust/Cargo.toml', 'tests/phase5a-baseline')
HERE = Path(__file__).resolve().parent


def repo_root(start=None):
    """Return the nearest ancestor of ``start`` that is the MosDNS repository.

    ``start`` may be a directory or a file inside it. A directory qualifies only
    when every marker in ``MARKERS`` exists there; anything else raises instead
    of falling back to a path that may not exist.
    """
    current = Path(start) if start is not None else HERE
    current = current if current.is_dir() else current.parent
    for candidate in (current, *current.parents):
        if all((candidate / marker).exists() for marker in MARKERS):
            return candidate
    raise ValueError(f'repository root with {"/".join(MARKERS)} not found above {current}')


REPO = repo_root()
