use orbit_core::{OrbitError, OrbitRuntime};

pub(super) fn execute_doctor(runtime: &OrbitRuntime) -> Result<(), OrbitError> {
    use orbit_core::command::tool::DoctorStatus;

    let results = runtime.doctor()?;
    let mut issues = 0;

    use crate::output::table::{Column, Table};
    let mut table = Table::new(vec![
        Column::new("TOOL").fixed(),
        Column::new("STATUS").fixed(),
        Column::new("DETAILS"),
    ])
    .empty_message("no tools registered");
    for r in &results {
        let status_str = match r.status {
            DoctorStatus::Ok => "ok",
            DoctorStatus::Warning => "warning",
            DoctorStatus::Error => "ERROR",
        };
        if r.status != DoctorStatus::Ok {
            issues += 1;
        }
        use comfy_table::Cell;
        table.add_row(vec![
            Cell::new(&r.tool_name),
            crate::output::color::doctor_status_color_cell(status_str),
            Cell::new(&r.message),
        ]);
    }
    table.print();

    if issues == 0 {
        println!(
            "\n{}",
            crate::output::color::job_state_color("All tools healthy.")
        );
    } else {
        eprintln!("\n{} issue(s) found.", issues);
    }

    Ok(())
}
