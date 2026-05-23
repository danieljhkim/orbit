use clap::Args;
use orbit_core::skill_catalog::LoadedSkill;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct SkillListArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for SkillListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let skills = runtime.list_file_skills()?;
        if self.json {
            let values = skills.iter().map(skill_summary_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            let mut table = crate::output::table::build_table(&["ID", "HASH", "TAGS", "SUMMARY"]);
            for skill in skills {
                let summary = skill
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.summary.clone())
                    .unwrap_or_default();
                let tags = skill.meta.as_ref().map(|meta| meta.tags.len()).unwrap_or(0);
                table.add_row(vec![
                    skill.id.clone(),
                    skill.content_hash[..10].to_string(),
                    tags.to_string(),
                    summary,
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}

fn skill_summary_json(skill: &LoadedSkill) -> Value {
    json!({
        "id": skill.id,
        "content_hash": skill.content_hash,
        "path": skill.path,
        "meta": skill.meta,
    })
}
