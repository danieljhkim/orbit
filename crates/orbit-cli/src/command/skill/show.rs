use clap::Args;
use orbit_core::skill_catalog::LoadedSkill;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct SkillShowArgs {
    pub name: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for SkillShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let skill = runtime.show_file_skill(&self.name)?;
        if self.json {
            crate::output::json::print_pretty(&skill_to_json(&skill))
        } else {
            println!("Skill:         {}", skill.id);
            println!("Path:          {}", skill.path.display());
            println!("Content hash:  {}", skill.content_hash);
            println!("\nBehavioral Contract (SKILL.md):");
            println!("{}", skill.content);
            println!("\nStructured Metadata (meta.json):");
            match &skill.meta_raw {
                Some(value) => println!(
                    "{}",
                    serde_json::to_string_pretty(value)
                        .map_err(|e| OrbitError::Execution(e.to_string()))?
                ),
                None => println!("(none)"),
            }
            Ok(())
        }
    }
}

fn skill_to_json(skill: &LoadedSkill) -> Value {
    json!({
        "id": skill.id,
        "path": skill.path,
        "content_hash": skill.content_hash,
        "content": skill.content,
        "sections": {
            "purpose": skill.sections.purpose,
            "behavioral_constraints": skill.sections.behavioral_constraints,
            "output_requirements": skill.sections.output_requirements,
            "evaluation_focus": skill.sections.evaluation_focus,
            "prohibitions": skill.sections.prohibitions,
            "examples": skill.sections.examples,
        },
        "meta": skill.meta,
        "meta_raw": skill.meta_raw,
        "output_schema": skill.output_schema,
    })
}
