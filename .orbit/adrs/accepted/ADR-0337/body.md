## Context
Deterministic action names had independent core advertisement, core forwarding, engine constants, and engine dispatch lists. The resulting skew shipped asset actions that were not invocable.

## Decision
Declare action names and their core-or-engine ownership once in `orbit-common`, generating typed core and engine action enums plus parsing and advertised names. Core dispatches exhaustively by the shared type, while engine implementation dispatch exhaustively matches the engine enum.

## Consequences
- Adding a declared core or engine action without its respective dispatch arm fails compilation through a non-exhaustive match; implementations cannot name an undeclared typed action.
- Runtime assets still use string action names, and catalog coverage remains responsible for catching invalid external asset references.
- The redundant core dispatch asset-scan guard and its debug assertion are removed because the duplicated registries no longer exist.
- Cost: adding an action requires selecting its ownership in the common declaration, which deliberately makes the cross-crate boundary explicit.