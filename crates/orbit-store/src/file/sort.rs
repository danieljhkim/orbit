use chrono::{DateTime, Utc};

pub(crate) fn sort_by_created_desc_id_asc<T, FCreatedAt, FId>(
    items: &mut [T],
    created_at: FCreatedAt,
    id: FId,
) where
    FCreatedAt: Fn(&T) -> &DateTime<Utc>,
    FId: Fn(&T) -> &str,
{
    items.sort_by(|a, b| {
        created_at(b)
            .cmp(created_at(a))
            .then_with(|| id(a).cmp(id(b)))
    });
}
