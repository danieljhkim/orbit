use std::path::Path;

use orbit_types::OrbitError;

use crate::fs_utils::write_text_with_parent;

const DEFAULT_IDENTITY_FILES: [(&str, &str); 4] = [
    ("linus", include_str!("../../assets/identities/linus.yaml")),
    (
        "lamport",
        include_str!("../../assets/identities/lamport.yaml"),
    ),
    ("prii", include_str!("../../assets/identities/prii.yaml")),
    ("steve", include_str!("../../assets/identities/steve.yaml")),
];

pub(crate) fn seed_default_identities(
    identity_root: &Path,
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let mut count = 0usize;
    for (name, content) in DEFAULT_IDENTITY_FILES {
        let path = identity_root.join(format!("{name}.yaml"));
        if !overwrite && path.exists() {
            continue;
        }
        write_text_with_parent(&path, content)?;
        count += 1;
    }
    Ok(count)
}
