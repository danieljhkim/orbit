use orbit_common::types::{PolicyDef, UNRESTRICTED_FS_PROFILE};
use orbit_core::OrbitError;
use serde_json::{Value, json};

pub(super) fn policy_json(def: &PolicyDef) -> Result<Value, OrbitError> {
    Ok(json!({
        "name": def.name,
        "description": def.description,
        "deny_read": def.deny_read,
        "deny_modify": def.deny_modify,
        "fs_profiles": effective_profiles_json(def)?,
        "created_at": def.created_at.to_rfc3339(),
        "updated_at": def.updated_at.to_rfc3339(),
    }))
}

pub(super) fn print_policy(def: &PolicyDef) -> Result<(), OrbitError> {
    println!("Name:        {}", def.name);
    if let Some(desc) = &def.description {
        println!("Description: {desc}");
    }
    println!("Created:     {}", def.created_at.to_rfc3339());
    println!("Updated:     {}", def.updated_at.to_rfc3339());

    println!("\nGlobal Denies:");
    println!("  denyRead:   {}", render_rule_list(&def.deny_read));
    println!("  denyModify: {}", render_rule_list(&def.deny_modify));

    println!("\nfsProfiles:");
    for profile_name in sorted_profile_names(def) {
        let effective = def.effective_profile(&profile_name)?;
        println!("  {}:", profile_name);
        println!("    read:   {}", render_rule_list(&effective.read));
        println!("    modify: {}", render_rule_list(&effective.modify));
    }

    Ok(())
}

fn effective_profiles_json(def: &PolicyDef) -> Result<Value, OrbitError> {
    let mut profiles = Vec::new();
    for profile_name in sorted_profile_names(def) {
        let effective = def.effective_profile(&profile_name)?;
        profiles.push(json!({
            "name": profile_name,
            "read": effective.read,
            "modify": effective.modify,
        }));
    }
    Ok(Value::Array(profiles))
}

pub(super) fn sorted_profile_names(def: &PolicyDef) -> Vec<String> {
    let mut names: Vec<String> = def.fs_profiles.keys().cloned().collect();
    names.sort();
    if !names.iter().any(|name| name == UNRESTRICTED_FS_PROFILE) {
        names.push(UNRESTRICTED_FS_PROFILE.to_string());
    }
    names
}

fn render_rule_list(rules: &[String]) -> String {
    if rules.is_empty() {
        "[]".to_string()
    } else {
        rules.join(", ")
    }
}

pub(super) fn status_word(allowed: bool) -> &'static str {
    if allowed { "allowed" } else { "denied" }
}
