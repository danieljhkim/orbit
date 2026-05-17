#!/usr/bin/env python3
"""
Orbit Scoreboard Visualizer
Reads .orbit/state/scoreboard/{duel_plan,tokens,pr}.json
and renders an interactive HTML dashboard.
"""

import json
import sys
from pathlib import Path
from collections import defaultdict
from datetime import datetime

try:
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots
except ImportError:
    print("Installing plotly...")
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "plotly", "-q"])
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots


# ── Config ────────────────────────────────────────────────────────────────────

REPO_ROOT      = Path(__file__).resolve().parent.parent
SCOREBOARD_DIR = REPO_ROOT / ".orbit/state/scoreboard"
OUTPUT_HTML    = REPO_ROOT / ".orbit/state/scoreboard_trends.html"

AGENT_COLORS = {
    "claude":  "#4C9BE8",
    "codex":   "#F5A623",
    "gemini":  "#7ED321",
    "grok":    "#E84C9B",
    "system":  "#888888",
}
KNOWN_AGENTS = ("claude", "codex", "gemini", "grok")

def agent_color(name: str) -> str:
    return AGENT_COLORS.get(name.lower(), "#BD10E0")


def known_agents(*counters) -> list[str]:
    agents = set(KNOWN_AGENTS)
    for counter in counters:
        agents.update(counter.keys())
    return sorted(agents)


# ── Loaders ───────────────────────────────────────────────────────────────────

def load_json(path: Path) -> dict | list:
    try:
        return json.loads(path.read_text())
    except Exception:
        return {}


def agent_label(role: dict) -> str:
    return f"{role['agent']} / {role['model']}"


# ── Duel plan charts ──────────────────────────────────────────────────────────

def chart_win_rate(runs: list, fig, row, col):
    """Win/loss count per agent — one trace per agent so stacking works correctly."""
    wins = defaultdict(int)
    total = defaultdict(int)

    for r in runs:
        winner_agent = r["roles"][r["outcome"]["winner"]]["agent"]
        wins[winner_agent] += 1
        for role in ("planner_a", "planner_b"):
            total[r["roles"][role]["agent"]] += 1

    agents = known_agents(total)

    # One trace per agent so each bar is its own color in the stack
    for a in agents:
        color = agent_color(a)
        fig.add_trace(go.Bar(
            name=a,
            x=["Wins", "Losses"],
            y=[wins[a], total[a] - wins[a]],
            marker_color=color,
            showlegend=True,
        ), row=row, col=col)

    fig.update_yaxes(title_text="Duel count", row=row, col=col)


def chart_win_rate_pct(runs: list, fig, row, col):
    """Win % per agent as a gauge-style bar."""
    wins  = defaultdict(int)
    total = defaultdict(int)
    for r in runs:
        winner_agent = r["roles"][r["outcome"]["winner"]]["agent"]
        wins[winner_agent] += 1
        for role in ("planner_a", "planner_b"):
            total[r["roles"][role]["agent"]] += 1

    agents = known_agents(total)
    pcts   = [100 * wins[a] / total[a] if total[a] else 0 for a in agents]
    colors = [agent_color(a) for a in agents]

    fig.add_trace(go.Bar(
        x=agents, y=pcts,
        marker_color=colors,
        text=[f"{p:.0f}%" for p in pcts],
        textposition="outside",
        showlegend=False,
    ), row=row, col=col)
    fig.update_yaxes(title_text="Win rate (%)", range=[0, 110], row=row, col=col)


def chart_tool_calls(runs: list, fig, row, col):
    """Average tool calls per role per agent."""
    calls = defaultdict(list)   # agent → [tool_call_counts]

    for r in runs:
        for role in ("planner_a", "planner_b"):
            eff = r["efficiency"].get(role, {})
            tc  = eff.get("tool_call_count")
            if tc is not None:
                agent = r["roles"][role]["agent"]
                calls[agent].append(tc)

    agents = known_agents(calls)
    avgs   = [sum(calls[a]) / len(calls[a]) if calls[a] else 0 for a in agents]
    colors = [agent_color(a) for a in agents]

    fig.add_trace(go.Bar(
        x=agents, y=avgs,
        marker_color=colors,
        text=[f"{v:.1f}" for v in avgs],
        textposition="outside",
        showlegend=False,
    ), row=row, col=col)
    fig.update_yaxes(title_text="Avg tool calls", row=row, col=col)


def chart_wall_clock(runs: list, fig, row, col):
    """Wall clock time per planner per duel (scatter)."""
    for role in ("planner_a", "planner_b"):
        dates, times, labels = [], [], []
        for r in runs:
            eff = r["efficiency"].get(role, {})
            wc  = eff.get("wall_clock_ms")
            if wc is None:
                continue
            agent = r["roles"][role]["agent"]
            ts    = datetime.fromisoformat(r["completed_at"].replace("Z", "+00:00"))
            dates.append(ts)
            times.append(wc / 1000)
            labels.append(f"{agent} ({role})<br>{r['task_id']}")

        if not dates:
            continue

        # group by agent for coloring
        by_agent = defaultdict(lambda: ([], [], []))
        for d, t, l, r2 in zip(dates, times, labels, runs):
            a = r2["roles"][role]["agent"]
            by_agent[a][0].append(d)
            by_agent[a][1].append(t)
            by_agent[a][2].append(l)

        for agent, (xs, ys, ls) in by_agent.items():
            fig.add_trace(go.Scatter(
                x=xs, y=ys,
                mode="markers",
                name=f"{agent} ({role})",
                marker=dict(color=agent_color(agent), size=10,
                            symbol="circle" if role == "planner_a" else "diamond"),
                text=ls,
                hoverinfo="text+y",
                showlegend=True,
            ), row=row, col=col)

    fig.update_yaxes(title_text="Wall clock (s)", row=row, col=col)


def chart_token_breakdown(runs: list, fig, row, col):
    """Stacked token breakdown (input / cache_read / cache_create / output) per agent."""
    token_fields = ["input", "cache_read", "cache_create", "output"]
    field_colors = ["#4C9BE8", "#50E3C2", "#F5A623", "#7ED321"]

    totals = defaultdict(lambda: defaultdict(int))  # agent → field → total

    for r in runs:
        for role in ("planner_a", "planner_b"):
            agent = r["roles"][role]["agent"]
            tu = r["efficiency"].get(role, {}).get("token_usage", {})
            for f in token_fields:
                totals[agent][f] += tu.get(f, 0)

    agents = known_agents(totals)
    if not agents:
        return

    for f, color in zip(token_fields, field_colors):
        fig.add_trace(go.Bar(
            name=f,
            x=agents,
            y=[totals[a][f] for a in agents],
            marker_color=color,
        ), row=row, col=col)

    fig.update_layout(barmode="stack")
    fig.update_yaxes(title_text="Total tokens", row=row, col=col)


def chart_arbiter_breakdown(runs: list, fig, row, col):
    """How often each agent serves as arbiter."""
    counts = defaultdict(int)
    for r in runs:
        counts[r["roles"]["arbiter"]["agent"]] += 1

    agents = known_agents(counts)
    fig.add_trace(go.Pie(
        labels=agents,
        values=[counts[a] for a in agents],
        marker_colors=[agent_color(a) for a in agents],
        hole=0.4,
        showlegend=True,
        name="Arbiter",
    ), row=row, col=col)


# ── Tokens scoreboard chart ───────────────────────────────────────────────────

def chart_tokens_scoreboard(data: dict, fig, row, col):
    """Agent-level token totals from tokens.json."""
    agents_data = [a for a in data.get("agents", []) if a.get("total_tokens", 0) > 0]
    if not agents_data:
        fig.add_annotation(
            text="No token data yet<br>(non-Claude providers emit zeros)",
            xref="paper", yref="paper", x=0.5, y=0.5,
            showarrow=False, font=dict(size=13, color="#888"),
            row=row, col=col,
        )
        return

    agents = [f"{a['agent']} / {a['model']}" for a in agents_data]
    totals = [a["total_tokens"] for a in agents_data]
    colors = [agent_color(a["agent"]) for a in agents_data]

    fig.add_trace(go.Bar(
        x=agents, y=totals,
        marker_color=colors,
        showlegend=False,
    ), row=row, col=col)
    fig.update_yaxes(title_text="Total tokens", row=row, col=col)


# ── Main ──────────────────────────────────────────────────────────────────────

def main():
    if not SCOREBOARD_DIR.exists():
        sys.exit(f"Scoreboard directory not found: {SCOREBOARD_DIR}")

    duel_data     = load_json(SCOREBOARD_DIR / "duel_plan.json")
    tokens_data   = load_json(SCOREBOARD_DIR / "tokens.json")
    # pr.json is currently empty

    runs = duel_data.get("runs", []) if isinstance(duel_data, dict) else []

    if not runs:
        print("WARNING: No duel_plan runs found — charts will be empty.")

    print(f"Duel runs: {len(runs)}")

    fig = make_subplots(
        rows=3, cols=2,
        subplot_titles=[
            "Wins & Losses by Agent",
            "Win Rate (%)",
            "Avg Tool Calls per Planner",
            "Wall Clock Time per Duel (s)",
            "Token Breakdown by Agent (all duels)",
            "Arbiter Role Distribution",
        ],
        specs=[
            [{"type": "xy"},  {"type": "xy"}],
            [{"type": "xy"},  {"type": "xy"}],
            [{"type": "xy"},  {"type": "domain"}],
        ],
        vertical_spacing=0.13,
        horizontal_spacing=0.10,
    )

    if runs:
        chart_win_rate(runs,         fig, row=1, col=1)
        chart_win_rate_pct(runs,     fig, row=1, col=2)
        chart_tool_calls(runs,       fig, row=2, col=1)
        chart_wall_clock(runs,       fig, row=2, col=2)
        chart_token_breakdown(runs,  fig, row=3, col=1)
        chart_arbiter_breakdown(runs,fig, row=3, col=2)

        # tokens.json only tracks Claude invocations; skip if empty

    fig.update_layout(
        title=dict(
            text=f"Orbit Scoreboard &nbsp;·&nbsp; {len(runs)} planning duels",
            font=dict(size=18),
        ),
        height=1050,
        template="plotly_dark",
        legend=dict(orientation="v", x=1.02, y=1),
        margin=dict(t=80, r=200),
        barmode="stack",
    )

    OUTPUT_HTML.parent.mkdir(parents=True, exist_ok=True)
    fig.write_html(str(OUTPUT_HTML), include_plotlyjs="cdn")
    print(f"Saved → {OUTPUT_HTML}")

    if sys.stdout.isatty():
        fig.show()


if __name__ == "__main__":
    main()
