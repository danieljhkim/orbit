#!/usr/bin/env python3
"""
Orbit Task Dashboard
Reads all task.yaml files from .orbit/tasks/ and renders an interactive HTML dashboard.
"""

import sys
from pathlib import Path
from collections import defaultdict
from datetime import datetime, timezone

try:
    import yaml
except ImportError:
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "pyyaml", "-q", "--break-system-packages"])
    import yaml

try:
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots
except ImportError:
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "plotly", "-q", "--break-system-packages"])
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots


# ── Config ────────────────────────────────────────────────────────────────────

REPO_ROOT   = Path(__file__).resolve().parent.parent
TASKS_DIR   = REPO_ROOT / ".orbit/tasks"
OUTPUT_HTML = REPO_ROOT / ".orbit/state/tasks_dashboard.html"

STATUS_ORDER = ["proposed", "backlog", "in_progress", "review", "blocked",
                "done", "archived", "rejected", "someday"]

STATUS_COLORS = {
    "proposed":    "#4C9BE8",
    "backlog":     "#888888",
    "in_progress": "#F5A623",
    "review":      "#BD10E0",
    "blocked":     "#D0021B",
    "done":        "#7ED321",
    "archived":    "#50E3C2",
    "rejected":    "#B8860B",
    "someday":     "#555577",
}

TYPE_COLORS = {
    "feature":  "#4C9BE8",
    "bug":      "#D0021B",
    "issue":    "#F5A623",
    "friction": "#BD10E0",
    "chore":    "#888888",
}

PRIORITY_ORDER = ["critical", "high", "medium", "low", "none"]
PRIORITY_COLORS = {
    "critical": "#D0021B",
    "high":     "#F5A623",
    "medium":   "#4C9BE8",
    "low":      "#7ED321",
    "none":     "#888888",
}

AGENT_COLORS = {
    "claude":  "#4C9BE8",
    "codex":   "#F5A623",
    "gemini":  "#7ED321",
    "human":   "#50E3C2",
}

def agent_color(name: str | None) -> str:
    if not name:
        return "#888888"
    for k, v in AGENT_COLORS.items():
        if k in name.lower():
            return v
    return "#888888"


# ── Loading ───────────────────────────────────────────────────────────────────

def load_all_tasks(base: Path) -> list[dict]:
    tasks = []
    for task_file in base.rglob("task.yaml"):
        try:
            data = yaml.safe_load(task_file.read_text())
            if not isinstance(data, dict):
                continue

            # Resolve status from directory name
            parts = task_file.parts
            # path: .orbit/tasks/<status>/[month/]<id>/task.yaml
            status_idx = next(i for i, p in enumerate(parts) if p == "tasks") + 1
            status = parts[status_idx]
            data["_status"] = status

            # Parse timestamps
            for field in ("created_at", "updated_at"):
                val = data.get(field)
                if isinstance(val, str):
                    try:
                        data[f"_{field}"] = datetime.fromisoformat(val.replace("Z", "+00:00"))
                    except Exception:
                        data[f"_{field}"] = None
                elif isinstance(val, datetime):
                    data[f"_{field}"] = val.replace(tzinfo=timezone.utc) if val.tzinfo is None else val
                else:
                    data[f"_{field}"] = None

            data["_created_date"] = data["_created_at"].date() if data["_created_at"] else None

            # Resolve priority
            data["_priority"] = (data.get("priority") or "none").lower()

            # Resolve type
            data["_type"] = (data.get("type") or "unknown").lower()

            # Resolve created_by agent name
            cb = data.get("created_by") or ""
            data["_created_by_agent"] = cb.split("/")[0].strip().lower() if cb else "unknown"

            # Resolve implemented_by
            ib = data.get("implemented_by") or ""
            data["_implemented_by"] = ib.split("/")[0].strip().lower() if ib else None

            tasks.append(data)
        except Exception:
            pass
    return tasks


# ── Charts ────────────────────────────────────────────────────────────────────

def chart_status_breakdown(tasks, fig, row, col):
    counts = defaultdict(int)
    for t in tasks:
        counts[t["_status"]] += 1

    statuses = [s for s in STATUS_ORDER if counts[s] > 0]
    values   = [counts[s] for s in statuses]
    colors   = [STATUS_COLORS.get(s, "#888") for s in statuses]

    fig.add_trace(go.Bar(
        x=statuses, y=values,
        marker_color=colors,
        text=values, textposition="outside",
        showlegend=False,
    ), row=row, col=col)
    fig.update_yaxes(title_text="Tasks", row=row, col=col)


def chart_type_breakdown(tasks, fig, row, col):
    counts = defaultdict(int)
    for t in tasks:
        counts[t["_type"]] += 1

    types  = sorted(counts, key=lambda x: -counts[x])
    values = [counts[t] for t in types]
    colors = [TYPE_COLORS.get(t, "#888") for t in types]

    fig.add_trace(go.Bar(
        x=types, y=values,
        marker_color=colors,
        text=values, textposition="outside",
        showlegend=False,
    ), row=row, col=col)
    fig.update_yaxes(title_text="Tasks", row=row, col=col)


def chart_tasks_created_over_time(tasks, fig, row, col):
    by_date = defaultdict(int)
    for t in tasks:
        if t["_created_date"]:
            by_date[t["_created_date"]] += 1

    dates  = sorted(by_date)
    counts = [by_date[d] for d in dates]

    # Cumulative line
    cumulative = []
    running = 0
    for c in counts:
        running += c
        cumulative.append(running)

    fig.add_trace(go.Bar(
        x=dates, y=counts,
        name="Created / day",
        marker_color="#4C9BE8",
        opacity=0.6,
        showlegend=True,
    ), row=row, col=col)
    fig.add_trace(go.Scatter(
        x=dates, y=cumulative,
        name="Cumulative",
        mode="lines",
        line=dict(color="#F5A623", width=2),
        yaxis="y2",
        showlegend=True,
    ), row=row, col=col)
    fig.update_yaxes(title_text="Created / day", row=row, col=col)


def chart_priority_distribution(tasks, fig, row, col):
    # Priority × status heatmap-style stacked bar
    by_priority = defaultdict(lambda: defaultdict(int))
    for t in tasks:
        by_priority[t["_priority"]][t["_status"]] += 1

    priorities = [p for p in PRIORITY_ORDER if by_priority[p]]
    active_statuses = [s for s in STATUS_ORDER if any(by_priority[p][s] for p in priorities)]

    for status in active_statuses:
        fig.add_trace(go.Bar(
            name=status,
            x=priorities,
            y=[by_priority[p][status] for p in priorities],
            marker_color=STATUS_COLORS.get(status, "#888"),
            showlegend=True,
        ), row=row, col=col)

    fig.update_yaxes(title_text="Tasks", row=row, col=col)


def chart_creator_breakdown(tasks, fig, row, col):
    counts = defaultdict(int)
    for t in tasks:
        counts[t["_created_by_agent"]] += 1

    agents = sorted(counts, key=lambda x: -counts[x])
    values = [counts[a] for a in agents]
    colors = [agent_color(a) for a in agents]

    fig.add_trace(go.Bar(
        x=agents, y=values,
        marker_color=colors,
        text=values, textposition="outside",
        showlegend=False,
    ), row=row, col=col)
    fig.update_yaxes(title_text="Tasks created", row=row, col=col)


def chart_implementer_breakdown(tasks, fig, row, col):
    counts = defaultdict(int)
    for t in tasks:
        ib = t["_implemented_by"]
        if ib:
            counts[ib] += 1

    if not counts:
        return

    agents = sorted(counts, key=lambda x: -counts[x])
    values = [counts[a] for a in agents]
    colors = [agent_color(a) for a in agents]

    fig.add_trace(go.Bar(
        x=agents, y=values,
        marker_color=colors,
        text=values, textposition="outside",
        showlegend=False,
    ), row=row, col=col)
    fig.update_yaxes(title_text="Tasks implemented", row=row, col=col)


def chart_throughput(tasks, fig, row, col):
    """Done tasks per day (completion rate)."""
    done_tasks = [t for t in tasks if t["_status"] in ("done", "archived")]
    by_date = defaultdict(int)
    for t in done_tasks:
        if t["_created_date"]:
            by_date[t["_created_date"]] += 1

    if not by_date:
        return

    dates  = sorted(by_date)
    counts = [by_date[d] for d in dates]

    fig.add_trace(go.Scatter(
        x=dates, y=counts,
        mode="lines+markers",
        name="Done / day",
        line=dict(color="#7ED321", width=2),
        fill="tozeroy",
        fillcolor="rgba(126,211,33,0.15)",
        showlegend=False,
    ), row=row, col=col)
    fig.update_yaxes(title_text="Completed tasks", row=row, col=col)


# ── Main ──────────────────────────────────────────────────────────────────────

def main():
    if not TASKS_DIR.exists():
        sys.exit(f"Tasks directory not found: {TASKS_DIR}")

    tasks = load_all_tasks(TASKS_DIR)
    if not tasks:
        sys.exit("No tasks found.")

    print(f"Loaded {len(tasks)} tasks")
    for s in STATUS_ORDER:
        n = sum(1 for t in tasks if t["_status"] == s)
        if n:
            print(f"  {s}: {n}")

    fig = make_subplots(
        rows=4, cols=2,
        subplot_titles=[
            "Tasks by Status",
            "Tasks by Type",
            "Tasks Created Over Time",
            "Priority × Status",
            "Created by Agent",
            "Implemented by Agent",
            "Completion Throughput (done + archived per day)",
            "",
        ],
        specs=[
            [{"type": "xy"}, {"type": "xy"}],
            [{"type": "xy"}, {"type": "xy"}],
            [{"type": "xy"}, {"type": "xy"}],
            [{"type": "xy", "colspan": 2}, None],
        ],
        vertical_spacing=0.10,
        horizontal_spacing=0.10,
    )

    chart_status_breakdown(tasks,          fig, row=1, col=1)
    chart_type_breakdown(tasks,            fig, row=1, col=2)
    chart_tasks_created_over_time(tasks,   fig, row=2, col=1)
    chart_priority_distribution(tasks,     fig, row=2, col=2)
    chart_creator_breakdown(tasks,         fig, row=3, col=1)
    chart_implementer_breakdown(tasks,     fig, row=3, col=2)
    chart_throughput(tasks,                fig, row=4, col=1)

    active = sum(1 for t in tasks if t["_status"] in ("proposed", "backlog", "in_progress", "review", "blocked"))
    done   = sum(1 for t in tasks if t["_status"] in ("done", "archived"))

    fig.update_layout(
        title=dict(
            text=f"Orbit Task Dashboard &nbsp;·&nbsp; {len(tasks)} total &nbsp;·&nbsp; {active} active &nbsp;·&nbsp; {done} completed",
            font=dict(size=18),
        ),
        height=1400,
        template="plotly_dark",
        barmode="stack",
        legend=dict(orientation="v", x=1.02, y=1),
        margin=dict(t=80, r=200),
    )

    OUTPUT_HTML.parent.mkdir(parents=True, exist_ok=True)
    fig.write_html(str(OUTPUT_HTML), include_plotlyjs="cdn")
    print(f"Saved → {OUTPUT_HTML}")

    if sys.stdout.isatty():
        fig.show()


if __name__ == "__main__":
    main()
