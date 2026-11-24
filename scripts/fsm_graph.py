"""Render exact protocol exports and explicitly lossy component projections."""
import json
from collections import defaultdict
from pathlib import Path


def load_graph(path):
    graph = {"nodes": [], "edges": []}
    seen_header = seen_summary = False
    with Path(path).open() as stream:
        for line in stream:
            record = json.loads(line)
            kind = record.pop("type")
            if kind == "header":
                if seen_header or record["schemaVersion"] != 1:
                    raise ValueError("duplicate header or unsupported export version")
                graph.update(record)
                seen_header = True
            elif kind == "summary":
                if seen_summary:
                    raise ValueError("duplicate export summary")
                graph["summary"] = record
                seen_summary = True
            elif kind in ("node", "edge"):
                graph[kind + "s"].append(record)
            else:
                raise ValueError(f"unknown record type: {kind}")
    if not seen_header or not seen_summary:
        raise ValueError("unfinished export: header or final summary is missing")
    graph["nodes"].sort(key=lambda n: n["id"])
    if [n["id"] for n in graph["nodes"]] != list(range(len(graph["nodes"]))):
        raise ValueError("node IDs must be unique and contiguous")
    for edge in graph["edges"]:
        if not 0 <= edge["source"] < len(graph["nodes"]):
            raise ValueError("unknown edge source")
        if edge["target"] is not None and not 0 <= edge["target"] < len(graph["nodes"]):
            raise ValueError("unknown edge target")
        if not 0 <= edge["event"] < len(graph["events"]):
            raise ValueError("unknown edge event")
    if (graph["summary"]["states"] != len(graph["nodes"]) or
            graph["summary"]["edges"] != len(graph["edges"])):
        raise ValueError("export counts disagree with graph records")
    return graph


def quote(value):
    return json.dumps(str(value), ensure_ascii=False)


def event_name(event):
    value = event["value"]
    return value if isinstance(value, str) else next(iter(value))


def event_label(event):
    return event["label"].replace("KvellModel.Event.", "").replace("KvellModel.Kind.", "")


def caption(graph, overview=False):
    summary = graph["summary"]
    completeness = "Complete for these bounds" if summary["complete"] else "INCOMPLETE: exploration limit reached"
    bounds = ", ".join(f"{key}={value}" for key, value in graph["bounds"].items())
    title = "Observed component transitions" if overview else "Reachable protocol states"
    result = f'{title}\n{summary["states"]:,} states / {summary["edges"]:,} transitions — {completeness}\n{bounds}'
    if overview:
        result += "\nProjection: IDs and other components are merged; paths may not be concrete executions."
    return result


def node_label(node):
    state = node["state"]
    requests = ", ".join(f'r{i}:{r["kind"]}/{r["phase"]}' for i, r in enumerate(state["worker"]["requests"]))
    buffers = ", ".join(f'b{i}:{cache_phase(p)}' for i, p in enumerate(state["cache"]["buffers"]))
    slots = ", ".join(f'c{i}:{io_phase(c)}' for i, c in enumerate(state["io"]["slots"]))
    return f'S{node["id"]}\n{requests}\n{buffers}\n{slots}'


def full_dot(graph):
    yield "digraph protocol {"
    yield '  graph [rankdir=LR, labelloc=t, fontname="sans-serif"];'
    yield f'  label={quote(caption(graph))};'
    yield '  node [shape=box, style="rounded,filled", fillcolor="#f1f5f9", fontname="sans-serif"];'
    yield '  edge [fontname="sans-serif", fontsize=9];'
    yield '  start [shape=point]; start -> s0;'
    for node in graph["nodes"]:
        color = "#fee2e2" if node["violations"] or node["deadlock"] else "#dcfce7" if node["quiescent"] else "#f1f5f9"
        if not node["expanded"] or node["omittedEdges"]:
            color = "#fef3c7"
        tooltip = json.dumps(node, ensure_ascii=False, separators=(",", ":"))
        yield f'  s{node["id"]} [label={quote(node_label(node))}, fillcolor={quote(color)}, tooltip={quote(tooltip)}];'
    if graph["summary"]["omittedEdges"]:
        yield '  omitted [shape=octagon, fillcolor="#fef3c7", label="Omitted states\\nstate limit reached"];'
    for edge in graph["edges"]:
        target = "omitted" if edge["target"] is None else f's{edge["target"]}'
        label = event_label(graph["events"][edge["event"]])
        yield f'  s{edge["source"]} -> {target} [label={quote(label)}];'
    yield "}"


def cache_phase(page):
    return "absent" if page is None else "loading" if not page["ready"] else "dirty" if page["dirty"] else "clean"


def io_phase(ctx):
    if ctx is None:
        return "free"
    stage = {"prepared": "SQ", "submitted": "kernel", "completed": "CQ"}[ctx["stage"]]
    result = f'{ctx["kind"]} / {stage}'
    if ctx["stage"] == "completed":
        result += " / success" if ctx["success"] else " / error or short"
    return result


def components(state):
    return {
        "Worker": [(r, "fresh" if r["phase"] == "fresh" else f'{r["kind"]} / {r["phase"]}') for r in state["worker"]["requests"]],
        "Page cache": [(p, cache_phase(p)) for p in state["cache"]["buffers"]],
        "I/O transport": [(c, io_phase(c)) for c in state["io"]["slots"]],
        "Kernel file image": [(p, "cells: " + "".join("1" if x else "0" for x in p)) for p in state["kernel"]["disk"]],
    }


def overview_dot(graph):
    views = [components(n["state"]) for n in graph["nodes"]]
    nodes, transitions = defaultdict(set), defaultdict(set)
    for view in views:
        for group, entries in view.items():
            nodes[group].update(label for _, label in entries)
    for edge in graph["edges"]:
        if edge["target"] is None:
            continue  # No invented transitions through omitted states.
        old, new = views[edge["source"]], views[edge["target"]]
        for group in old:
            for (a, source), (b, target) in zip(old[group], new[group]):
                if a != b:
                    transitions[group, source, target].add(event_name(graph["events"][edge["event"]]))
    yield "digraph components {"
    yield '  graph [rankdir=LR, labelloc=t, fontname="sans-serif", bgcolor="white", pad=0.4];'
    yield f'  label={quote(caption(graph, overview=True))};'
    yield '  node [shape=box, style="rounded,filled", fontname="sans-serif", fontsize=11];'
    yield '  edge [fontname="sans-serif", fontsize=9, color="#64748b"];'
    colors = ["#dbeafe", "#dcfce7", "#ffedd5", "#ede9fe"]
    for i, (group, labels) in enumerate(nodes.items()):
        yield f'  subgraph cluster_{i} {{'
        yield f'    label={quote(group)}; color="#cbd5e1";'
        ids = {label: f"g{i}n{j}" for j, label in enumerate(sorted(labels))}
        for label, name in ids.items():
            yield f'    {name} [label={quote(label)}, fillcolor={quote(colors[i])}];'
        for (owner, source, target), events in sorted(transitions.items()):
            if owner == group:
                yield f'    {ids[source]} -> {ids[target]} [label={quote(chr(10).join(sorted(events)))}];'
        yield "  }"
    yield "}"


def write_graphs(graph, prefix):
    prefix = Path(prefix)
    prefix.parent.mkdir(parents=True, exist_ok=True)
    Path(str(prefix) + ".json").write_text(json.dumps(graph, ensure_ascii=False, separators=(",", ":")) + "\n")
    for suffix, renderer in [(".dot", full_dot), (".overview.dot", overview_dot)]:
        with Path(str(prefix) + suffix).open("w") as out:
            for line in renderer(graph):
                out.write(line + "\n")
