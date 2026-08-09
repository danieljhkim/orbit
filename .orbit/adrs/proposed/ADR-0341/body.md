## Context
The job executor depended on a dispatcher host and a separate deterministic/task/environment/run host family, both implemented by OrbitRuntime. Keeping both families with a documented ownership rule was a real alternative, but it would preserve two call graphs and let capabilities drift between them.

## Decision
Declare one orbit-engine RuntimeHost capability boundary with one OrbitRuntime implementation. The engine parses the shared typed deterministic-action declaration: engine-owned actions execute directly, while core-owned actions cross RuntimeHost once. Orbit-core owns workflow-admission policy.

## Consequences
- A deterministic engine action no longer round-trips through orbit-core before reaching its engine implementation.
- The boundary is readable in one declaration and has one production implementor.
- Cost: the single trait is broad, and focused test hosts must rely on defaults or implement the capabilities they exercise instead of choosing among smaller public host traits.