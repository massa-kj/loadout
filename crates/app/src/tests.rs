use super::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// --- Fixture helpers ---

/// Write a file, creating all parent directories.
fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Make a file executable on Unix.
#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

/// Build an AppContext whose dirs and local_root all point inside `tmp`.
fn make_ctx(tmp: &TempDir) -> AppContext {
    let root = tmp.path().to_path_buf();
    let config_home = root.join("config");
    AppContext {
        platform: platform::detect_platform(),
        local_root: config_home.clone(),
        dirs: platform::Dirs {
            config_home,
            data_home: root.join("data"),
            state_home: root.join("state"),
            cache_home: root.join("cache"),
        },
        sources_override: None,
    }
}

/// Write a minimal script-mode component to `{local_root}/components/{name}/`.
/// Creates platform-appropriate scripts: .sh on Linux/WSL, .ps1 on Windows.
fn write_script_component(root: &Path, name: &str) {
    let feat_dir = root.join("config").join("components").join(name);
    write(
        &feat_dir.join("component.yaml"),
        "spec_version: 1\nmode: script\n",
    );

    let platform = platform::detect_platform();
    match platform {
        platform::Platform::Windows => {
            // PowerShell scripts
            let install_ps1 = feat_dir.join("install.ps1");
            write(&install_ps1, "exit 0\n");
            let uninstall_ps1 = feat_dir.join("uninstall.ps1");
            write(&uninstall_ps1, "exit 0\n");
        }
        platform::Platform::Linux | platform::Platform::Wsl => {
            // Shell scripts
            let install_sh = feat_dir.join("install.sh");
            write(&install_sh, "#!/usr/bin/env sh\nexit 0\n");
            make_executable(&install_sh);
            let uninstall_sh = feat_dir.join("uninstall.sh");
            write(&uninstall_sh, "#!/usr/bin/env sh\nexit 0\n");
            make_executable(&uninstall_sh);
        }
    }
}

/// Write a minimal config.yaml referencing the given component names.
/// Components must be canonical `source_id/name` form; they are grouped by source_id.
/// No strategy section is written (uses Strategy::default()).
fn write_config(dir: &Path, filename: &str, components: &[&str]) -> PathBuf {
    // Group components by source_id.
    let mut grouped: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for f in components {
        let (source, name) = f
            .split_once('/')
            .expect("component must be canonical source/name");
        grouped.entry(source).or_default().push(name);
    }
    let mut components_str = String::new();
    for (source, names) in &grouped {
        components_str.push_str(&format!("    {source}:\n"));
        for name in names {
            components_str.push_str(&format!("      {name}: {{}}\n"));
        }
    }
    let content = format!("profile:\n  components:\n{components_str}");
    let path = dir.join(filename);
    write(&path, &content);
    path
}

/// Collect all events emitted during apply.
fn collect_apply(
    ctx: &AppContext,
    config_path: &Path,
) -> (Result<ExecutorReport, AppError>, Vec<Event>) {
    let mut events = vec![];
    let result = apply(ctx, config_path, &mut |e| events.push(e));
    (result, events)
}

// --- Tests ---

/// Missing config returns ConfigNotFound.
#[test]
fn plan_missing_config_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    let config_path = tmp.path().join("nonexistent.yaml");

    let err = plan(&ctx, &config_path).unwrap_err();
    assert!(
        matches!(err, AppError::ConfigNotFound { .. }),
        "expected ConfigNotFound, got {err:?}"
    );
}

/// Config with unrecognised components: recognised list is empty → plan has no actions.
#[test]
fn plan_unknown_components_produce_empty_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    // Component referenced in config does not exist in index → desired IDs empty.
    let config_path = write_config(tmp.path(), "config.yaml", &["local/nonexistent"]);

    // Should succeed: empty desired produces a plan with no actions.
    let p = plan(&ctx, &config_path).unwrap();
    assert!(
        p.actions.is_empty(),
        "plan should have no actions for unknown components"
    );
}

/// plan() with a valid script component returns a Plan with a Create action.
#[test]
fn plan_script_component_returns_create_action() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_component(tmp.path(), "git");
    let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

    let p = plan(&ctx, &config_path).unwrap();
    assert_eq!(p.actions.len(), 1);
    let action = &p.actions[0];
    assert_eq!(action.component.as_str(), "local/git");
    assert!(matches!(action.operation, model::plan::Operation::Create));
}

/// apply() installs a script component and commits state.
#[test]
fn apply_script_component_commits_state() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_component(tmp.path(), "git");
    let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

    let (result, events) = collect_apply(&ctx, &config_path);

    let report = result.unwrap();
    assert_eq!(report.executed.len(), 1, "expected one component executed");
    assert!(report.failed.is_empty());

    // State file must be committed.
    assert!(ctx.state_path().exists(), "state.json must be written");

    // Events: ComponentStart + ComponentDone.
    let starts = events
        .iter()
        .filter(|e| matches!(e, Event::ComponentStart { .. }))
        .count();
    let dones = events
        .iter()
        .filter(|e| matches!(e, Event::ComponentDone { .. }))
        .count();
    assert_eq!(starts, 1);
    assert_eq!(dones, 1);
}

/// apply() a second time on an already-installed component emits no actions (noop).
#[test]
fn apply_already_installed_component_is_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_component(tmp.path(), "git");
    let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

    // First apply: installs.
    let (r1, _) = collect_apply(&ctx, &config_path);
    r1.unwrap();

    // Second apply: state already reflects desired; should be a noop.
    let (r2, events2) = collect_apply(&ctx, &config_path);
    let report2 = r2.unwrap();

    // No actions executed: component is already in state.
    assert!(
        report2.executed.is_empty(),
        "second apply should have no executed components"
    );
    // No events at all (no actions → no ComponentStart/Done).
    let start_count = events2
        .iter()
        .filter(|e| matches!(e, Event::ComponentStart { .. }))
        .count();
    assert_eq!(start_count, 0, "no ComponentStart events on noop");
}

/// apply() missing config propagates ConfigNotFound.
#[test]
fn apply_missing_config_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    let config_path = tmp.path().join("does_not_exist.yaml");

    let (result, _) = collect_apply(&ctx, &config_path);
    assert!(matches!(
        result.unwrap_err(),
        AppError::ConfigNotFound { .. }
    ));
}

/// apply() two script components: both install, state has both.
#[test]
fn apply_multiple_components_all_installed() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_component(tmp.path(), "git");
    write_script_component(tmp.path(), "node");
    let config_path = write_config(tmp.path(), "config.yaml", &["local/git", "local/node"]);

    let (result, _) = collect_apply(&ctx, &config_path);
    let report = result.unwrap();

    assert_eq!(report.executed.len(), 2);
    assert!(report.failed.is_empty());
}

/// apply() removes a component that is in state but not in the config.
#[test]
fn apply_removes_undesired_component_from_state() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_component(tmp.path(), "git");
    write_script_component(tmp.path(), "node");

    // First apply: install git + node.
    let config_both = write_config(tmp.path(), "both.yaml", &["local/git", "local/node"]);
    collect_apply(&ctx, &config_both).0.unwrap();

    // Second apply: only git desired → node should be destroyed.
    let config_git_only = write_config(tmp.path(), "git_only.yaml", &["local/git"]);
    let (result, _) = collect_apply(&ctx, &config_git_only);
    let report = result.unwrap();

    // One action executed (Destroy node).
    assert_eq!(report.executed.len(), 1);
    assert!(report.failed.is_empty());

    // Reload state from disk and verify.
    let state = state::load(&ctx.state_path()).unwrap();
    assert!(
        state.components.contains_key("local/git"),
        "git must still be in state"
    );
    assert!(
        !state.components.contains_key("local/node"),
        "node must be removed from state"
    );
}

/// Config without a strategy section → Strategy::default() is used (no error).
#[test]
fn plan_without_strategy_section_uses_default_strategy() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_component(tmp.path(), "git");
    let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

    // write_config omits the strategy section → Strategy::default() is used.
    let p = plan(&ctx, &config_path).unwrap();
    assert_eq!(p.actions.len(), 1);
}

/// apply() with a script component whose uninstall fails is non-fatal;
/// other components in the same run still succeed.
#[test]
fn apply_failing_uninstall_is_non_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);

    // Component with a failing uninstall script.
    let feat_dir = tmp
        .path()
        .join("config")
        .join("components")
        .join("badcomponent");
    write(
        &feat_dir.join("component.yaml"),
        "spec_version: 1\nmode: script\n",
    );

    let platform = platform::detect_platform();
    match platform {
        platform::Platform::Windows => {
            // PowerShell scripts
            let install_ps1 = feat_dir.join("install.ps1");
            write(&install_ps1, "exit 0\n");
            let uninstall_ps1 = feat_dir.join("uninstall.ps1");
            write(&uninstall_ps1, "exit 1\n"); // Always fails
        }
        platform::Platform::Linux | platform::Platform::Wsl => {
            // Shell scripts
            let install_sh = feat_dir.join("install.sh");
            write(&install_sh, "#!/usr/bin/env sh\nexit 0\n");
            make_executable(&install_sh);
            let uninstall_sh = feat_dir.join("uninstall.sh");
            write(&uninstall_sh, "#!/usr/bin/env sh\nexit 1\n"); // Always fails
            make_executable(&uninstall_sh);
        }
    }

    // A good component that succeeds.
    write_script_component(tmp.path(), "git");

    // First apply: install both.
    let config_both = write_config(
        tmp.path(),
        "both.yaml",
        &["local/badcomponent", "local/git"],
    );
    collect_apply(&ctx, &config_both).0.unwrap();

    // Second apply: only git desired → badcomponent must be destroyed (fails), git is noop.
    let config_git_only = write_config(tmp.path(), "git.yaml", &["local/git"]);
    let (result, events) = collect_apply(&ctx, &config_git_only);
    let report = result.unwrap(); // Must not be a fatal error.

    // badcomponent destruction failed → shows up in failed list.
    assert_eq!(report.failed.len(), 1, "badcomponent uninstall should fail");
    // git was already installed; no new action.
    assert!(report.executed.is_empty(), "git is already installed");

    // A ComponentFailed event is emitted.
    let ff_count = events
        .iter()
        .filter(|e| matches!(e, Event::ComponentFailed { .. }))
        .count();
    assert_eq!(ff_count, 1);
}
