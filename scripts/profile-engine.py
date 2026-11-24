#!/usr/bin/env python3
"""Run the single engine profiling test in separate measurement passes.

Examples:
  python3 scripts/profile-engine.py
  python3 scripts/profile-engine.py --collect timing
  ENGINE_PROFILE_PAGES=16384 python3 scripts/profile-engine.py --collect syscalls
  ENGINE_PROFILE_DIR=/path/on/ssd python3 scripts/profile-engine.py --collect all

Requires Python 3 and Cargo. Optional collectors require strace or perf on PATH.
No tools are installed and no kernel permissions/settings are changed. The kernel
pass records system-wide events so it can see asynchronous io_uring workers and
block completions; it may require additional tracing privileges. Run it on an
otherwise idle machine. Its trace can contain activity from other processes.

The test uses aligned O_DIRECT I/O by default. ENGINE_PROFILE_DIRECT=0 measures
buffered I/O instead (including warm page-cache reads). CPU cache misses and the
filesystem page cache are different: perf counters measure the former; filemap
events in the kernel trace help investigate the latter, without claiming a
universal page-cache hit rate. Valgrind/Gungraun is not required for this native
kernel I/O workload. Each collector runs the exact same test in a fresh process.
"""

import argparse
from datetime import datetime, timezone
import html
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]
TEST = "io::engine::tests::profile_engine"


def run(command, log, env, timeout):
    print(f"Running: {' '.join(map(str, command))}", flush=True)
    with log.open("w") as output:
        process = subprocess.Popen(
            command, cwd=ROOT, env=env, stdout=output,
            stderr=subprocess.STDOUT, start_new_session=True,
        )
        try:
            code = process.wait(timeout=timeout)
        except (subprocess.TimeoutExpired, KeyboardInterrupt) as error:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            output.write(f"\nStopped: {type(error).__name__}\n")
            if isinstance(error, KeyboardInterrupt):
                raise
            code = 124
    return {
        "status": "ok" if code == 0 else "failed",
        "exit_code": code,
        "command": [str(arg) for arg in command],
        "log": log.name,
    }


def workload_metrics(log):
    for line in log.read_text().splitlines():
        if line.startswith("ENGINE_PROFILE "):
            return json.loads(line.removeprefix("ENGINE_PROFILE "))
    return None


def report(directory, data):
    (directory / "summary.json").write_text(json.dumps(data, indent=2) + "\n")
    parts = [
        '<!doctype html><meta charset="utf-8"><title>Engine profile</title>',
        '<style>body{font:16px system-ui;max-width:1100px;margin:40px auto;padding:0 20px}'
        'pre{background:#f3f4f6;padding:16px;overflow:auto}li{margin:8px 0}</style>',
        '<h1>Engine profile</h1>',
        '<p>One test workload, separate measurement passes. Use timing results for '
        'throughput; tracing changes execution speed. External collectors include '
        'test startup, setup, verification, sync and cleanup. The kernel pass is '
        'system-wide and can include unrelated processes. Hardware CPU/cache '
        'counters use user-space events; kernel work is captured by the trace pass.</p>',
        '<p>Default: direct I/O, 4 KiB pages, queue depth up to 32. Timed phases '
        'include buffer preparation and verification; file synchronization is '
        'reported separately. Buffered reads may hit the page cache. These are '
        'single-run observations, not statistical regression estimates.</p>',
        '<h2>Results and collection status</h2>',
        '<pre>' + html.escape(json.dumps(data, indent=2)) + '</pre>',
        '<h2>Artifacts</h2><ul>',
    ]
    for path in sorted(directory.iterdir()):
        if path.name == "index.html":
            continue
        name = html.escape(path.name, quote=True)
        parts.append(f'<li><a href="{name}">{name}</a></li>')
    parts.append('</ul>')
    if (directory / "kernel.perf.data").exists():
        command = f"perf script -i {directory / 'kernel.perf.data'}"
        parts.append('<h2>Inspect kernel events</h2><pre>' + html.escape(command) + '</pre>')
    for name in ("syscalls.txt", "counters.csv"):
        path = directory / name
        if path.exists():
            parts.append(f'<h2>{name}</h2><pre>{html.escape(path.read_text())}</pre>')
    (directory / "index.html").write_text("\n".join(parts))
    print(f"Report: {directory / 'index.html'}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--collect", choices=("all", "timing", "syscalls", "counters", "kernel"), default="all")
    parser.add_argument("--timeout", type=int, default=120, help="maximum seconds per workload pass")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    os.chdir(ROOT)
    directory = ROOT / "target" / "engine-profile" / str(time.time_ns())
    directory.mkdir(parents=True)
    env = dict(os.environ, CARGO_PROFILE_RELEASE_DEBUG="1")
    data = {
        "test": TEST,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(),
        "settings": {key: value for key, value in env.items() if key.startswith("ENGINE_PROFILE_")},
        "passes": {},
    }
    build = run(
        ["cargo", "test", "--release", "--lib", "--no-run", "--message-format=json"],
        directory / "build.log", env, 600,
    )
    data["build"] = build
    if build["status"] != "ok":
        report(directory, data)
        return 1
    binaries = []
    for line in (directory / "build.log").read_text().splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (item.get("reason") == "compiler-artifact" and item.get("executable")
                and item.get("profile", {}).get("test")
                and "lib" in item.get("target", {}).get("kind", [])):
            binaries.append(item["executable"])
    if len(binaries) != 1:
        data["error"] = f"Expected one library test executable, found {len(binaries)}"
        report(directory, data)
        return 1
    test = [binaries[0], TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
    timing_log = directory / "timing.log"
    timing = run(test, timing_log, env, args.timeout)
    timing["metrics"] = workload_metrics(timing_log)
    if timing["metrics"] is None:
        timing["status"] = "failed"
    data["passes"]["timing"] = timing
    if timing["status"] != "ok":
        report(directory, data)
        return 1

    collectors = {
        "syscalls": ["strace", "-f", "-c", "-o", str(directory / "syscalls.txt"), "--"],
        "counters": [
            "perf", "stat", "-x", ";", "-o", str(directory / "counters.csv"),
            "-e", "task-clock,context-switches,page-faults,cycles:u,instructions:u,"
            "cache-references:u,cache-misses:u,branches:u,branch-misses:u", "--",
        ],
        "kernel": [
            "perf", "record", "--all-cpus", "-o", str(directory / "kernel.perf.data"),
            "-e", "sched:sched_switch", "-e", "sched:sched_wakeup",
            "-e", "io_uring:*", "-e", "block:block_rq_issue", "-e", "block:block_rq_complete",
            "-e", "filemap:*", "--",
        ],
    }
    for name, command in collectors.items():
        if args.collect not in ("all", name):
            continue
        if shutil.which(command[0]) is None:
            result = {"status": "unavailable", "reason": f"{command[0]} is not on PATH"}
        else:
            result = run(command + test, directory / f"{name}.log", env, args.timeout)
            if result["status"] == "ok" and workload_metrics(directory / f"{name}.log") is None:
                result.update(status="failed", reason="No workload metrics were emitted")
            if name == "counters" and result["status"] == "ok":
                counters = directory / "counters.csv"
                if not counters.exists() or any(
                    marker in counters.read_text() for marker in ("<not supported>", "<not counted>")
                ):
                    result.update(status="partial", reason="Some requested counters were unavailable; inspect counters.csv")
        data["passes"][name] = result
        print(f"{name}: {result['status']}", flush=True)
    report(directory, data)
    # Missing collectors must not silently look like a complete successful run.
    return 0 if all(result["status"] == "ok" for result in data["passes"].values()) else 1


if __name__ == "__main__":
    sys.exit(main())
