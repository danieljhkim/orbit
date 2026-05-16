mod planning_duel;
mod record_scores;
mod select_roles;
mod select_task;

use orbit_common::types::OrbitError;

pub(super) use planning_duel::run_planning_duel;
pub(super) use record_scores::record_duel_scores;
pub(super) use select_roles::select_duel_roles;
pub(super) use select_task::select_duel_task;

fn role_permutation_at(family_count: usize, index: usize) -> Result<[usize; 3], OrbitError> {
    if family_count < 3 {
        return Err(OrbitError::Execution(format!(
            "duel role selection requires at least 3 agent families, got {family_count}"
        )));
    }

    let total = family_count * (family_count - 1) * (family_count - 2);
    let mut remaining = index % total;
    for first in 0..family_count {
        for second in 0..family_count {
            if second == first {
                continue;
            }
            for third in 0..family_count {
                if third == first || third == second {
                    continue;
                }
                if remaining == 0 {
                    return Ok([first, second, third]);
                }
                remaining -= 1;
            }
        }
    }

    Err(OrbitError::Execution(
        "duel role permutation enumeration failed".to_string(),
    ))
}

fn validate_role_permutation(
    perm: [usize; 3],
    family_count: usize,
    action: &str,
) -> Result<[usize; 3], OrbitError> {
    if let Some(index) = perm.into_iter().find(|index| *index >= family_count) {
        return Err(OrbitError::Execution(format!(
            "{action} produced family index {index}, but only {family_count} families are configured"
        )));
    }
    if perm[0] == perm[1] || perm[0] == perm[2] || perm[1] == perm[2] {
        return Err(OrbitError::Execution(format!(
            "{action} produced non-distinct family indices: {perm:?}"
        )));
    }
    Ok(perm)
}

#[cfg(test)]
mod tests {
    use orbit_common::types::all_agent_families;

    use super::*;

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
}
