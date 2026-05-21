use orbit_common::types::all_agent_families;

use super::super::*;

#[test]
fn role_permutations_cover_every_family() {
    let family_count = all_agent_families().len();
    let total = family_count * (family_count - 1) * (family_count - 2);
    let mut seen = vec![false; family_count];

    for index in 0..total {
        let perm = role_permutation_at(family_count, index).expect("valid permutation");
        for family_index in perm {
            seen[family_index] = true;
        }
    }

    assert_eq!(seen, vec![true; family_count]);
}
