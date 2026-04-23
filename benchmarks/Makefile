.PHONY: help graph-run graph-run-claude graph-run-codex graph-sweep graph-sweep-dry-run graph-sweep-claude graph-sweep-codex graph-aggregate graph-test

PYTHON ?= python3
BENCH_ROOT := $(CURDIR)
GRAPH_DIR := $(BENCH_ROOT)/graph

GRAPH_PROVIDER ?= claude
GRAPH_ARM ?= no-graph
GRAPH_TASK ?= locate-agentruntime
GRAPH_TASKS ?=
GRAPH_SEED ?= 1
GRAPH_N ?= 1
GRAPH_BUDGET ?= 1.0
GRAPH_ARMS ?= no-graph graph-only hybrid
GRAPH_SWEEP_ID ?=
GRAPH_RUN_ARGS ?=
GRAPH_SWEEP_ARGS ?=

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
	@echo "    Print aggregate benchmark tables."
	@echo ""
	@echo "  make -C benchmarks graph-test"
	@echo "    Run benchmark script unit tests."

graph-run:
	$(PYTHON) $(GRAPH_DIR)/scripts/run.py \
		$(GRAPH_ARM) \
		$(GRAPH_TASK) \
		$(GRAPH_SEED) \
		--provider $(GRAPH_PROVIDER) \
		--budget $(GRAPH_BUDGET) \
		$(GRAPH_RUN_ARGS)

graph-run-claude:
	$(MAKE) graph-run GRAPH_PROVIDER=claude

graph-run-codex:
	$(MAKE) graph-run GRAPH_PROVIDER=codex

graph-sweep:
	$(PYTHON) $(GRAPH_DIR)/scripts/sweep.py \
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
	$(PYTHON) $(GRAPH_DIR)/scripts/aggregate.py

graph-test:
	$(PYTHON) -m unittest discover -s $(GRAPH_DIR)/scripts -p 'test_*.py'
