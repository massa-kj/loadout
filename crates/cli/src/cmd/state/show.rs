// crates/cli/src/cmd/state/show.rs — `loadout state show` implementation

use std::process;

use crate::args::{OutputFormat, StateShowArgs};
use crate::context::build_app_context;

pub fn run(args: StateShowArgs) {
    let ctx = build_app_context();
    let state = state::load(&ctx.state_path()).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    match args.output.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&state).unwrap_or_else(|e| {
                eprintln!("error: failed to serialize state: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => print_state_text(&state),
    }
}

fn print_state_text(state: &state::State) {
    let feature_count = state.features.len();
    let resource_count: usize = state.features.values().map(|f| f.resources.len()).sum();

    println!("version:   {}", state.version);
    println!("features:  {feature_count}");
    println!("resources: {resource_count}");

    if feature_count == 0 {
        println!("\n(no features installed)");
        return;
    }

    println!();
    let mut features: Vec<(&String, &state::FeatureState)> = state.features.iter().collect();
    features.sort_by_key(|(id, _)| id.as_str());

    for (id, feature) in &features {
        let n = feature.resources.len();
        println!(
            "  {id:<40}  {} resource{}",
            n,
            if n == 1 { "" } else { "s" }
        );
    }
}
