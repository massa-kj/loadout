// crates/cli/src/output/plan.rs — Plan display formatting

use model::plan::Operation;

pub fn print_plan(plan: &model::Plan, verbose: bool) {
    let has_anything =
        !plan.actions.is_empty() || !plan.blocked.is_empty() || (verbose && !plan.noops.is_empty());

    if !has_anything {
        println!("Nothing to do.");
        return;
    }

    if !plan.actions.is_empty() {
        println!("Actions:");
        for action in &plan.actions {
            let op_label = match action.operation {
                Operation::Create => "create",
                Operation::Destroy => "destroy",
                Operation::Replace => "replace",
                Operation::ReplaceBackend => "replace-backend",
                Operation::Strengthen => "strengthen",
            };
            println!("  [{op_label}] {}", action.component.as_str());
        }
        println!();
    }

    if !plan.blocked.is_empty() {
        println!("Blocked:");
        for entry in &plan.blocked {
            println!("  [blocked] {}: {}", entry.component.as_str(), entry.reason);
        }
        println!();
    }

    if verbose && !plan.noops.is_empty() {
        println!("No-op (already up to date):");
        for entry in &plan.noops {
            println!("  [noop] {}", entry.component.as_str());
        }
        println!();
    }

    let s = &plan.summary;
    let total_action = s.create + s.destroy + s.replace + s.replace_backend + s.strengthen;
    print!("Summary: {total_action} action(s)");
    if s.blocked > 0 {
        print!(", {} blocked", s.blocked);
    }
    if verbose {
        print!(", {} noop", plan.noops.len());
    }
    println!();
}
