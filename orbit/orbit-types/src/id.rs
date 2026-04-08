/// A human-readable, deterministic identifier string (e.g. `"T20260321-192339"`).
///
/// This is a type alias rather than a newtype so it serializes and deserializes
/// as a plain string without needing custom serde impls. All IDs are generated
/// by the store layer (`orbit-store`) at creation time using a timestamp-based
/// scheme; callers never construct IDs directly.
pub type OrbitId = String;
