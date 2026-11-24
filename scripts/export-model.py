#!/usr/bin/env python3
"""Compile and explore Protocol.lean through its real Event.run transitions."""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from fsm_graph import load_graph, write_graphs

ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / "model"


def natural(value):
    value = int(value)
    if value < 0:
        raise argparse.ArgumentTypeError("expected a nonnegative integer")
    return value


def sources():
    return {str(p.relative_to(MODEL)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted((MODEL / "KvellModel").rglob("*.lean"))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name, default in [("pages", 1), ("buffers", 1), ("slots", 1), ("requests", 2), ("cells", 2)]:
        parser.add_argument("--" + name, type=natural, default=default)
    parser.add_argument("--max-states", type=natural, default=10000, help="0 means unlimited; truncation is recorded")
    parser.add_argument("--max-depth", type=natural, default=0, help="0 means unlimited")
    parser.add_argument("--out", type=Path, default=MODEL / ".lake/fsm/protocol", help="output filename prefix")
    parser.add_argument("--from-jsonl", type=Path, help="render an existing explorer export without rerunning Lean")
    parser.add_argument("--render", choices=["none", "overview", "full", "both"], default="overview")
    parser.add_argument("--render-timeout", type=natural, default=120)
    args = parser.parse_args()
    if args.render != "none" and not shutil.which("dot"):
        parser.error("Graphviz's dot is missing; enter the Nix shell or use --render none")
    if args.from_jsonl:
        graph = load_graph(args.from_jsonl)
        graph["source"] = {"jsonl": str(args.from_jsonl.resolve())}
    else:
        before = sources()
        subprocess.run(["lake", "build", "KvellModel.Export.Explore"], cwd=MODEL, check=True)
        with tempfile.TemporaryDirectory(prefix="kvellox-fsm-") as directory:
            raw = Path(directory) / "graph.jsonl"
            command = ["lake", "env", "lean", "--run", "KvellModel/Export/Main.lean", str(raw)]
            command += [str(getattr(args, key)) for key in
                        ["pages", "buffers", "slots", "requests", "cells", "max_states", "max_depth"]]
            subprocess.run(command, cwd=MODEL, check=True)
            graph = load_graph(raw)
        if before != sources():
            raise ValueError("Lean sources changed during exploration; rerun for a consistent export")
        graph["source"] = {"entry": "KvellModel/Protocol.lean", "scheduler": "KvellModel/Event.lean",
                           "sha256": before, "toolchain": (MODEL / "lean-toolchain").read_text().strip()}
    write_graphs(graph, args.out)
    summary = graph["summary"]
    print(json.dumps(summary, indent=2), flush=True)
    for kind in ("overview", "full"):
        if args.render in (kind, "both"):
            base = str(args.out) + (".overview" if kind == "overview" else "")
            subprocess.run(["dot", "-Tsvg", base + ".dot", "-o", base + ".svg"],
                           check=True, timeout=args.render_timeout or None)
            print(Path(base + ".svg").resolve(), flush=True)
    print(f"Exact graph: {Path(str(args.out) + '.dot').resolve()}")
    print(f"State details: {Path(str(args.out) + '.json').resolve()}")
    return 1 if summary["violationStates"] or summary["deadlockStates"] else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"export failed: {error}", file=sys.stderr)
        sys.exit(2)
