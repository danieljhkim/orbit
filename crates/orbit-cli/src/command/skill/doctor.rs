use clap::Args;
use orbit_core::command::skill::{SkillDoctorResult, SkillDoctorStatus};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;
use crate::output::color::{Domain, Role};

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
        use crate::output::table::{Column, Table};
        let mut table = Table::new(vec![
            Column::new("SKILL").fixed(),
            Column::new("STATUS").fixed(),
            Column::new("DETAILS"),
        ])
        .empty_message("no file skills installed");
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
                crate::output::color::cell(status, Domain::DoctorStatus),
                Cell::new(&row.message),
            ]);
        }
        table.print();

        if issues == 0 {
            println!(
                "\n{}",
                crate::output::color::text("All skills healthy.", Role::Ok)
            );
        } else {
            eprintln!("\n{} issue(s) found.", issues);
        }
        Ok(())
    }
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
