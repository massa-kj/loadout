// crates/cli/src/cmd/migrate.rs — deprecated top-level `loadout migrate` shim
//
// The implementation lives in cmd/state/migrate.rs.
// This file exists only so that main.rs can call cmd::state::migrate::run()
// through the deprecation path; it adds no logic of its own.
