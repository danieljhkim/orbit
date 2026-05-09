.PHONY: help graph-run graph-run-claude graph-run-codex graph-sweep graph-sweep-dry-run graph-sweep-claude graph-sweep-codex graph-aggregate graph-test identity-key-run graph-latency-fetch graph-latency-run graph-latency-sweep graph-latency-aggregate

PYTHON ?= python3
BENCH_ROOT := $(CURDIR)

# GRAPH_VERSION selects which version's tasks/ and runs/ to target.
#   v3 → living harness at benchmarks/graph/v3/    (default; round 3 in progress)
#   v1 → frozen snapshot at benchmarks/graph/v1/
#   v2 → frozen snapshot at benchmarks/graph/v2/
# See benchmarks/CONVENTIONS.md for versioning rules.
GRAPH_VERSION ?= v3

# Scripts live once under benchmarks/graph/scripts/ and are expected to stay
# backward-compatible with all frozen run records. If a future round needs
# to break back-compat (e.g. record schema change), copy scripts into that
# version's directory at freeze time and override GRAPH_SCRIPTS when
# reproducing it. See benchmarks/CONVENTIONS.md §Freezing a version.
GRAPH_SCRIPTS ?= $(BENCH_ROOT)/graph/scripts
GRAPH_DIR := $(BENCH_ROOT)/graph/$(GRAPH_VERSION)

GRAPH_PROVIDER ?= claude
GRAPH_ARM ?= no-graph
GRAPH_TASK ?= locate-agentruntime
GRAPH_TASKS ?=
GRAPH_SEED ?= 1
GRAPH_N ?= 1
GRAPH_ARMS ?= no-graph graph-only hybrid
GRAPH_SWEEP_ID ?=
GRAPH_RUN_ARGS ?=
GRAPH_SWEEP_ARGS ?=

IDENTITY_KEY_SCRIPTS ?= $(BENCH_ROOT)/identity-key/scripts

# graph-latency is a perf benchmark (kind: perf). Same versioning rules as
# graph; see benchmarks/CONVENTIONS.md.
GRAPH_LATENCY_VERSION ?= v1
GRAPH_LATENCY_SCRIPTS ?= $(BENCH_ROOT)/graph-latency/scripts
GRAPH_LATENCY_DIR := $(BENCH_ROOT)/graph-latency/$(GRAPH_LATENCY_VERSION)

GL_CORPUS ?=
GL_TOOL ?=
GL_PHASE ?= query
GL_SEED ?= 1
GL_QUERY_SHAPE ?= default
GL_RUN_ARGS ?=
GL_SWEEP_ARGS ?=
GL_FETCH_ARGS ?=

# Export version vars so the Python scripts resolve VERSION_ROOT correctly.
export GRAPH_VERSION
export GRAPH_LATENCY_VERSION

help:
	@echo "Benchmark Make Targets"
	@echo ""
	@echo "  make -C benchmarks graph-run"
	@echo "    Run one graph benchmark cell."
	@echo "    Vars: GRAPH_PROVIDER=claude|codex GRAPH_ARM=... GRAPH_TASK=... GRAPH_SEED=..."
	@echo ""
	@echo "  make -C benchmarks graph-sweep"
	@echo "    Run a graph sweep."
	@echo "    Vars: GRAPH_PROVIDER=claude|codex GRAPH_N=... GRAPH_TASKS=\"...\" GRAPH_ARMS=\"...\" GRAPH_SWEEP_ID=..."
	@echo ""
	@echo "  make -C benchmarks graph-sweep-dry-run"
	@echo "    Materialize sweep order without executing runs."
	@echo ""
	@echo "  make -C benchmarks graph-run-codex"
	@echo "  make -C benchmarks graph-run-claude"
	@echo "  make -C benchmarks graph-sweep-codex"
	@echo "  make -C benchmarks graph-sweep-claude"
	@echo ""
	@echo "  make -C benchmarks graph-aggregate"
	@echo "    Print aggregate benchmark tables for GRAPH_VERSION."
	@echo ""
	@echo "  make -C benchmarks graph-test"
	@echo "    Run benchmark script unit tests (living scripts only)."
	@echo ""
	@echo "  make -C benchmarks identity-key-run"
	@echo "    Rebuild the identity-key v1 scenario records."
	@echo ""
	@echo "  make -C benchmarks graph-latency-fetch"
	@echo "    Populate ~/.cache/orbit-bench/<corpus> from pinned upstream SHAs."
	@echo "    Vars: GRAPH_LATENCY_VERSION=v1 GL_FETCH_ARGS=\"--corpus python-medium\""
	@echo ""
	@echo "  make -C benchmarks graph-latency-run"
	@echo "    Run one graph-latency cell (corpus x tool x phase x seed)."
	@echo "    Vars: GL_CORPUS=python-medium GL_TOOL=graph.search GL_PHASE=query GL_SEED=1"
	@echo ""
	@echo "  make -C benchmarks graph-latency-sweep"
	@echo "    Run the full graph-latency matrix (or a subset via GL_SWEEP_ARGS)."
	@echo ""
	@echo "  make -C benchmarks graph-latency-aggregate"
	@echo "    Print primary + build-phase tables for GRAPH_LATENCY_VERSION."
	@echo ""
	@echo "Versioning:"
	@echo "  GRAPH_VERSION=v3 (default)          targets benchmarks/graph/v3/ (living)."
	@echo "  GRAPH_VERSION=v2                    targets benchmarks/graph/v2/ (frozen)."
	@echo "  GRAPH_VERSION=v1                    targets benchmarks/graph/v1/ (frozen)."
	@echo "  GRAPH_LATENCY_VERSION=v1 (default)  targets benchmarks/graph-latency/v1/ (living)."
	@echo "  See benchmarks/CONVENTIONS.md."

graph-run:
	$(PYTHON) $(GRAPH_SCRIPTS)/run.py \
		$(GRAPH_ARM) \
		$(GRAPH_TASK) \
		$(GRAPH_SEED) \
		--provider $(GRAPH_PROVIDER) \
		$(GRAPH_RUN_ARGS)

graph-run-claude:
	$(MAKE) graph-run GRAPH_PROVIDER=claude

graph-run-codex:
	$(MAKE) graph-run GRAPH_PROVIDER=codex

graph-sweep:
	$(PYTHON) $(GRAPH_SCRIPTS)/sweep.py \
		--provider $(GRAPH_PROVIDER) \
		$(if $(GRAPH_TASKS),--tasks $(GRAPH_TASKS),) \
		--n $(GRAPH_N) \
		$(if $(GRAPH_SWEEP_ID),--sweep-id $(GRAPH_SWEEP_ID),) \
		$(if $(GRAPH_ARMS),--arms $(GRAPH_ARMS),) \
		$(GRAPH_SWEEP_ARGS)

graph-sweep-dry-run:
	$(MAKE) graph-sweep GRAPH_SWEEP_ARGS="$(GRAPH_SWEEP_ARGS) --dry-run"

graph-sweep-claude:
	$(MAKE) graph-sweep GRAPH_PROVIDER=claude

graph-sweep-codex:
	$(MAKE) graph-sweep GRAPH_PROVIDER=codex

graph-aggregate:
	$(PYTHON) $(GRAPH_SCRIPTS)/aggregate.py \
		--runs $(GRAPH_DIR)/runs \
		--tasks $(GRAPH_DIR)/tasks

graph-test:
	$(PYTHON) -m unittest discover -s $(BENCH_ROOT)/graph/scripts -p 'test_*.py'

identity-key-run:
	$(IDENTITY_KEY_SCRIPTS)/run.sh

graph-latency-fetch:
	$(GRAPH_LATENCY_SCRIPTS)/fetch.sh \
		--version $(GRAPH_LATENCY_VERSION) \
		$(GL_FETCH_ARGS)

graph-latency-run:
	$(PYTHON) $(GRAPH_LATENCY_SCRIPTS)/run.py \
		--version $(GRAPH_LATENCY_VERSION) \
		--corpus $(GL_CORPUS) \
		--tool $(GL_TOOL) \
		--phase $(GL_PHASE) \
		--seed $(GL_SEED) \
		--query-shape $(GL_QUERY_SHAPE) \
		$(GL_RUN_ARGS)

graph-latency-sweep:
	$(PYTHON) $(GRAPH_LATENCY_SCRIPTS)/sweep.py \
		--version $(GRAPH_LATENCY_VERSION) \
		$(GL_SWEEP_ARGS)

graph-latency-aggregate:
	$(PYTHON) $(GRAPH_LATENCY_SCRIPTS)/aggregate.py \
		--runs $(GRAPH_LATENCY_DIR)/runs
