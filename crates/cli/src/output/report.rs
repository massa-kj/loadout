// crates/cli/src/output/report.rs — Apply execution report display formatting

use app::ExecutorReport;

/// Print the apply execution report.
///
/// Returns `true` if all features succeeded, `false` if any failed.
pub fn print_apply_report(report: &ExecutorReport) -> bool {
    println!();
    if report.failed.is_empty() {
        println!("Config applied successfully.");
    } else {
        println!("Config applied with errors.");
    }

    if !report.executed.is_empty() {
        println!();
        println!("Executed ({}):", report.executed.len());
        for f in &report.executed {
            println!("  {} [{}]", f.id, f.operation);
        }
    }

    if !report.failed.is_empty() {
        println!();
        println!("Failed ({}):", report.failed.len());
        for f in &report.failed {
            println!("  {} [{}]: {}", f.id, f.operation, f.error);
        }
        println!();
        return false;
    }

    true
}
