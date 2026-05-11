## Context
Letting the agent self-certify success is too weak for buildable coding tasks. Mechanical checks need to execute outside the conversational loop.

## Decision
Groundhog verifies mechanical success criteria at the checkpoint-success boundary and converts failures into retryable `FailureReport`s.

## Consequences
- Success is gated on workspace reality, not just agent confidence.
- A richer shared verifier can serve non-Groundhog code paths too.
- Cost: the current runner still uses its own thinner inline verifier, so this decision is only partially reflected in implementation.
