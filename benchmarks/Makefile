.PHONY: help identity-key-run knowledge-command-equivalence

BENCH_ROOT := $(CURDIR)

IDENTITY_KEY_SCRIPTS ?= $(BENCH_ROOT)/identity-key/scripts
KCE_MODE ?= compare
KCE_FIXTURE ?=
KCE_BASELINE ?=

help:
	@echo "Benchmark Make Targets"
	@echo ""
	@echo "  make -C benchmarks identity-key-run"
	@echo "    Rebuild the identity-key v1 scenario records."
	@echo ""
	@echo "  make -C benchmarks knowledge-command-equivalence \\"
	@echo "      KCE_MODE=capture|compare KCE_FIXTURE=/path/to/workspace \\"
	@echo "      KCE_BASELINE=/path/to/baseline"
	@echo "    Capture or compare byte-stable task and knowledge command output."

identity-key-run:
	$(IDENTITY_KEY_SCRIPTS)/run.sh

knowledge-command-equivalence:
	@test -n "$(KCE_FIXTURE)" || { echo "KCE_FIXTURE is required" >&2; exit 2; }
	@test -n "$(KCE_BASELINE)" || { echo "KCE_BASELINE is required" >&2; exit 2; }
	$(BENCH_ROOT)/knowledge-command-equivalence/run.sh \
		$(KCE_MODE) \
		$(KCE_FIXTURE) \
		$(KCE_BASELINE)
