use clap::{Args, Subcommand};
use orbit_core::command::init::{LinkResult, UnlinkResult};
use orbit_core::command::skill::{SkillDoctorResult, SkillDoctorStatus};
use orbit_core::skill_catalog::LoadedSkill;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct SkillCommand {
    #[command(subcommand)]
    pub command: SkillSubcommand,
}

impl Execute for SkillCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum SkillSubcommand {
    List(SkillListArgs),
    Show(SkillShowArgs),
    Doctor(SkillDoctorArgs),
    /// Re-create skill symlinks in .agents/skills/ and .claude/skills/
    Link(SkillLinkArgs),
    /// Remove skill symlinks from .agents/skills/ and .claude/skills/
    Unlink(SkillUnlinkArgs),
}

impl Execute for SkillSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            SkillSubcommand::List(args) => args.execute(runtime),
            SkillSubcommand::Show(args) => args.execute(runtime),
            SkillSubcommand::Doctor(args) => args.execute(runtime),
            SkillSubcommand::Link(args) => args.execute(runtime),
            SkillSubcommand::Unlink(args) => args.execute(runtime),
        }
    }
}

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

#[derive(Args)]
pub struct SkillDoctorArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for SkillDoctorArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let rows = runtime.doctor_file_skills()?;
        if self.json {
            let values = rows.iter().map(doctor_row_json).collect::<Vec<_>>();
            return crate::output::json::print_pretty(&Value::Array(values));
        }

        let mut issues = 0usize;
        let mut table = crate::output::table::build_table(&["SKILL", "STATUS", "DETAILS"]);
        for row in &rows {
            let status = match row.status {
                SkillDoctorStatus::Ok => "ok",
                SkillDoctorStatus::Warning => "warning",
                SkillDoctorStatus::Error => "ERROR",
            };
            if row.status != SkillDoctorStatus::Ok {
                issues += 1;
            }
            use comfy_table::Cell;
            table.add_row(vec![
                Cell::new(&row.skill_name),
                crate::output::color::doctor_status_color_cell(status),
                Cell::new(&row.message),
            ]);
        }
        println!("{table}");

        if issues == 0 {
            println!(
                "\n{}",
                crate::output::color::job_state_color("All skills healthy.")
            );
        } else {
            eprintln!("\n{} issue(s) found.", issues);
        }
        Ok(())
    }
}

#[derive(Args)]
pub struct SkillLinkArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for SkillLinkArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = orbit_core::command::init::link_skills(&runtime.data_root())?;
        if self.json {
            crate::output::json::print_pretty(&link_result_json(&result))
        } else {
            if result.linked_count == 0 {
                println!("Skill symlinks are already up to date.");
            } else {
                println!("Linked {} skill(s) in:", result.linked_count);
                for root in &result.roots {
                    println!("  {}", root.display());
                }
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct SkillUnlinkArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for SkillUnlinkArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = orbit_core::command::init::unlink_skills(&runtime.data_root())?;
        if self.json {
            crate::output::json::print_pretty(&unlink_result_json(&result))
        } else {
            if result.removed_count == 0 {
                println!("No skill symlinks found to remove.");
            } else {
                println!("Removed {} skill symlink(s).", result.removed_count);
            }
            if !result.cleaned_dirs.is_empty() {
                println!("Cleaned up empty directories:");
                for dir in &result.cleaned_dirs {
                    println!("  {}", dir.display());
                }
            }
            Ok(())
        }
    }
}

fn link_result_json(result: &LinkResult) -> Value {
    json!({
        "linked_count": result.linked_count,
        "roots": result.roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    })
}

fn unlink_result_json(result: &UnlinkResult) -> Value {
    json!({
        "removed_count": result.removed_count,
        "cleaned_dirs": result.cleaned_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    })
}

fn skill_summary_json(skill: &LoadedSkill) -> Value {
    json!({
        "id": skill.id,
        "content_hash": skill.content_hash,
        "path": skill.path,
        "meta": skill.meta,
    })
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

fn doctor_row_json(row: &SkillDoctorResult) -> Value {
    json!({
        "skill_id": row.skill_name,
        "status": match row.status {
            SkillDoctorStatus::Ok => "ok",
            SkillDoctorStatus::Warning => "warning",
            SkillDoctorStatus::Error => "error",
        },
        "message": row.message,
    })
}
