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

/// Write a minimal script-mode feature to `{local_root}/features/{name}/`.
/// Creates platform-appropriate scripts: .sh on Linux/WSL, .ps1 on Windows.
fn write_script_feature(root: &Path, name: &str) {
    let feat_dir = root.join("config").join("features").join(name);
    write(
        &feat_dir.join("feature.yaml"),
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

/// Write a minimal config.yaml referencing the given feature names.
/// Features must be canonical `source_id/name` form; they are grouped by source_id.
/// No strategy section is written (uses Strategy::default()).
fn write_config(dir: &Path, filename: &str, features: &[&str]) -> PathBuf {
    // Group features by source_id.
    let mut grouped: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for f in features {
        let (source, name) = f
            .split_once('/')
            .expect("feature must be canonical source/name");
        grouped.entry(source).or_default().push(name);
    }
    let mut features_str = String::new();
    for (source, names) in &grouped {
        features_str.push_str(&format!("    {source}:\n"));
        for name in names {
            features_str.push_str(&format!("      {name}: {{}}\n"));
        }
    }
    let content = format!("profile:\n  features:\n{features_str}");
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

/// Config with unrecognised features: recognised list is empty → plan has no actions.
#[test]
fn plan_unknown_features_produce_empty_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    // Feature referenced in config does not exist in index → desired IDs empty.
    let config_path = write_config(tmp.path(), "config.yaml", &["local/nonexistent"]);

    // Should succeed: empty desired produces a plan with no actions.
    let p = plan(&ctx, &config_path).unwrap();
    assert!(
        p.actions.is_empty(),
        "plan should have no actions for unknown features"
    );
}

/// plan() with a valid script feature returns a Plan with a Create action.
#[test]
fn plan_script_feature_returns_create_action() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_feature(tmp.path(), "git");
    let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

    let p = plan(&ctx, &config_path).unwrap();
    assert_eq!(p.actions.len(), 1);
    let action = &p.actions[0];
    assert_eq!(action.feature.as_str(), "local/git");
    assert!(matches!(action.operation, model::plan::Operation::Create));
}

/// apply() installs a script feature and commits state.
#[test]
fn apply_script_feature_commits_state() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_feature(tmp.path(), "git");
    let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

    let (result, events) = collect_apply(&ctx, &config_path);

    let report = result.unwrap();
    assert_eq!(report.executed.len(), 1, "expected one feature executed");
    assert!(report.failed.is_empty());

    // State file must be committed.
    assert!(ctx.state_path().exists(), "state.json must be written");

    // Events: FeatureStart + FeatureDone.
    let starts = events
        .iter()
        .filter(|e| matches!(e, Event::FeatureStart { .. }))
        .count();
    let dones = events
        .iter()
        .filter(|e| matches!(e, Event::FeatureDone { .. }))
        .count();
    assert_eq!(starts, 1);
    assert_eq!(dones, 1);
}

/// apply() a second time on an already-installed feature emits no actions (noop).
#[test]
fn apply_already_installed_feature_is_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_feature(tmp.path(), "git");
    let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

    // First apply: installs.
    let (r1, _) = collect_apply(&ctx, &config_path);
    r1.unwrap();

    // Second apply: state already reflects desired; should be a noop.
    let (r2, events2) = collect_apply(&ctx, &config_path);
    let report2 = r2.unwrap();

    // No actions executed: feature is already in state.
    assert!(
        report2.executed.is_empty(),
        "second apply should have no executed features"
    );
    // No events at all (no actions → no FeatureStart/Done).
    let start_count = events2
        .iter()
        .filter(|e| matches!(e, Event::FeatureStart { .. }))
        .count();
    assert_eq!(start_count, 0, "no FeatureStart events on noop");
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

/// apply() two script features: both install, state has both.
#[test]
fn apply_multiple_features_all_installed() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_feature(tmp.path(), "git");
    write_script_feature(tmp.path(), "node");
    let config_path = write_config(tmp.path(), "config.yaml", &["local/git", "local/node"]);

    let (result, _) = collect_apply(&ctx, &config_path);
    let report = result.unwrap();

    assert_eq!(report.executed.len(), 2);
    assert!(report.failed.is_empty());
}

/// apply() removes a feature that is in state but not in the config.
#[test]
fn apply_removes_undesired_feature_from_state() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_feature(tmp.path(), "git");
    write_script_feature(tmp.path(), "node");

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
        state.features.contains_key("local/git"),
        "git must still be in state"
    );
    assert!(
        !state.features.contains_key("local/node"),
        "node must be removed from state"
    );
}

/// Config without a strategy section → Strategy::default() is used (no error).
#[test]
fn plan_without_strategy_section_uses_default_strategy() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);
    write_script_feature(tmp.path(), "git");
    let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

    // write_config omits the strategy section → Strategy::default() is used.
    let p = plan(&ctx, &config_path).unwrap();
    assert_eq!(p.actions.len(), 1);
}

/// apply() with a script feature whose uninstall fails is non-fatal;
/// other features in the same run still succeed.
#[test]
fn apply_failing_uninstall_is_non_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = make_ctx(&tmp);

    // Feature with a failing uninstall script.
    let feat_dir = tmp
        .path()
        .join("config")
        .join("features")
        .join("badfeature");
    write(
        &feat_dir.join("feature.yaml"),
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

    // A good feature that succeeds.
    write_script_feature(tmp.path(), "git");

    // First apply: install both.
    let config_both = write_config(tmp.path(), "both.yaml", &["local/badfeature", "local/git"]);
    collect_apply(&ctx, &config_both).0.unwrap();

    // Second apply: only git desired → badfeature must be destroyed (fails), git is noop.
    let config_git_only = write_config(tmp.path(), "git.yaml", &["local/git"]);
    let (result, events) = collect_apply(&ctx, &config_git_only);
    let report = result.unwrap(); // Must not be a fatal error.

    // badfeature destruction failed → shows up in failed list.
    assert_eq!(report.failed.len(), 1, "badfeature uninstall should fail");
    // git was already installed; no new action.
    assert!(report.executed.is_empty(), "git is already installed");

    // A FeatureFailed event is emitted.
    let ff_count = events
        .iter()
        .filter(|e| matches!(e, Event::FeatureFailed { .. }))
        .count();
    assert_eq!(ff_count, 1);
}
