/// Merge behavior requested by a registry payload.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MergeClass {
    /// Replays ordered records.
    Replay,
    /// Merges counter state using CRDT semantics.
    CrdtCounter,
    /// Appends immutable records in publication order.
    AppendOnly,
    /// Publishes immutable bytes addressed by content identity.
    ContentAddressedImmutable,
}
