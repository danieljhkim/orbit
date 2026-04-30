#!/usr/bin/env python3
import json
import os
import re

SUMMARY_PATH = ".orbit/state/scoreboard/summary.json"
README_PATH = "README.md"

if not os.path.exists(SUMMARY_PATH):
    exit(0)

with open(SUMMARY_PATH, "r") as f:
    data = json.load(f)

agents = data.get("agents", {})
# sort by tasks_completed desc
sorted_agents = sorted(agents.items(), key=lambda x: x[1].get("tasks_completed", 0), reverse=True)

markdown = "| Agent | Tasks | Task Review | Friction (R/A/Rej) | Tokens (Tot/Out) | Duels (W/L) | PR (Cm/Cln/Rev) |\n"
markdown += "|---|---|---|---|---|---|---|\n"

for name, metrics in sorted_agents:
    tasks = metrics.get("tasks_completed", 0)
    task_review = metrics.get("task_review", {})
    t_task_review = task_review.get("messages", 0)

    frict = metrics.get("friction", {})
    t_frict = f"{frict.get('reported', 0)}/{frict.get('accepted', 0)}/{frict.get('rejected', 0)}"
    
    toks = metrics.get("tokens", {})
    t_toks = f"{toks.get('total', 0)}/{toks.get('output', 0)}"
    
    duels = metrics.get("duels", {})
    t_duels = f"{duels.get('wins', 0)}/{duels.get('losses', 0)}"
    
    pr = metrics.get("pr", {})
    t_pr = f"{pr.get('review_comments', 0)}/{pr.get('merged_clean', 0)}/{pr.get('merged_with_revision', 0)}"
    
    markdown += f"| **{name}** | {tasks} | {t_task_review} | {t_frict} | {t_toks} | {t_duels} | {t_pr} |\n"

with open(README_PATH, "r") as f:
    readme = f.read()

pattern = re.compile(r'(<!-- SCOREBOARD_START -->).*?(<!-- SCOREBOARD_END -->)', re.DOTALL)

if pattern.search(readme):
    new_readme = pattern.sub(f'\\1\n\n{markdown.strip()}\n\n\\2', readme)
    with open(README_PATH, "w") as f:
        f.write(new_readme)
