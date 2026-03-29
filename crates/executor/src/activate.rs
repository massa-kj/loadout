//! Activation script generator.
//!
//! Converts an [`ExecutionEnvPlan`] into a shell-specific activation fragment
//! that can be evaluated in the user's interactive shell to expose the
//! execution environment (PATH changes, tool shims, etc.).
//!
//! # Supported shells
//!
//! | [`ShellKind`]         | Evaluation command                  |
//! |-----------------------|-------------------------------------|
//! | [`ShellKind::Bash`]   | `eval "$(loadout activate bash)"`   |
//! | [`ShellKind::Zsh`]    | `eval "$(loadout activate zsh)"`    |
//! | [`ShellKind::Fish`]   | `loadout activate fish \| source`   |
//! | [`ShellKind::PowerShell`] | `Invoke-Expression (& loadout activate pwsh)` |
//!
//! # Output contract
//!
//! - Only `export VAR=value` / `set -x VAR value` / `$env:VAR = ...` statements
//!   are emitted. No aliases, functions, or comments unless explicitly requested.
//! - Values are shell-escaped. Callers must not post-process the output.
//! - Variables with empty values are skipped.
//! - Output is deterministic: variables are emitted in sorted order.
//!
//! See: `tmp/20260322_backend-activate問題2.md`

use model::env::ExecutionEnvPlan;

// ---------------------------------------------------------------------------
// ShellKind
// ---------------------------------------------------------------------------

/// The target shell for activation script generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

// ---------------------------------------------------------------------------
// generate_activation
// ---------------------------------------------------------------------------

/// Generate a shell activation script from `plan` for the given `shell`.
///
/// The output is a string suitable for evaluation in the target shell.
/// It sets every variable present in `plan.vars`.
/// Variables with empty string values are omitted.
///
/// Output is deterministic (sorted by variable name) so it can be compared
/// in tests and diffs.
pub fn generate_activation(plan: &ExecutionEnvPlan, shell: ShellKind) -> String {
    let mut lines = Vec::new();

    // Sorted iteration over BTreeMap is already deterministic.
    for (key, value) in &plan.vars {
        if value.is_empty() {
            continue;
        }
        let line = match shell {
            ShellKind::Bash | ShellKind::Zsh => {
                // POSIX-style export: escape single-quote characters in value.
                format!("export {}={}", key, sh_quote(value))
            }
            ShellKind::Fish => {
                // Fish uses `set -x` for environment variables.
                format!("set -x {} {}", key, sh_quote(value))
            }
            ShellKind::PowerShell => {
                // PowerShell: $env:VAR = 'value'
                format!("$env:{} = {}", key, ps_quote(value))
            }
        };
        lines.push(line);
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Shell quoting helpers
// ---------------------------------------------------------------------------

/// Quote a value for POSIX sh / bash / zsh / fish using single-quote syntax.
///
/// Single-quoted strings in POSIX sh cannot contain single quotes directly;
/// they are escaped by ending the quoted string, inserting `\'`, and starting
/// a new quoted string: `foo'bar` → `'foo'\''bar'`.
fn sh_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    // Fast path: no special characters.
    if !value.contains([
        '\'', ' ', '"', '\\', '$', '`', '!', '\n', '\t', ';', '&', '|', '<', '>', '(', ')', '{',
        '}', '#', '~', '*', '?', '[', ']',
    ]) {
        return value.to_string();
    }

    // General case: wrap in single quotes, escaping embedded single quotes.
    let escaped = value.replace('\'', r"'\''");
    format!("'{escaped}'")
}

/// Quote a value for PowerShell using single-quote syntax.
///
/// PowerShell single-quoted strings treat everything literally except `'`,
/// which is doubled: `'foo''bar'` represents `foo'bar`.
fn ps_quote(value: &str) -> String {
    let escaped = value.replace('\'', "''");
    format!("'{escaped}'")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use model::env::ExecutionEnvPlan;
    use std::collections::BTreeMap;

    fn plan(vars: &[(&str, &str)]) -> ExecutionEnvPlan {
        ExecutionEnvPlan {
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn bash_exports_sorted_vars() {
        let p = plan(&[("PATH", "/usr/bin:/bin"), ("EDITOR", "vim")]);
        let out = generate_activation(&p, ShellKind::Bash);
        let lines: Vec<&str> = out.lines().collect();
        // Sorted: EDITOR before PATH.
        assert_eq!(lines[0], "export EDITOR=vim");
        assert_eq!(lines[1], "export PATH=/usr/bin:/bin");
    }

    #[test]
    fn zsh_same_as_bash() {
        let p = plan(&[("FOO", "bar")]);
        let bash = generate_activation(&p, ShellKind::Bash);
        let zsh = generate_activation(&p, ShellKind::Zsh);
        assert_eq!(bash, zsh);
    }

    #[test]
    fn fish_set_x_syntax() {
        let p = plan(&[("PATH", "/opt/bin:/usr/bin")]);
        let out = generate_activation(&p, ShellKind::Fish);
        assert_eq!(out, "set -x PATH /opt/bin:/usr/bin");
    }

    #[test]
    fn powershell_env_syntax() {
        let p = plan(&[("PATH", r"C:\tools\bin;C:\windows\system32")]);
        let out = generate_activation(&p, ShellKind::PowerShell);
        assert!(out.starts_with("$env:PATH = '"), "got: {out}");
    }

    #[test]
    fn skips_empty_values() {
        let p = plan(&[("EMPTY", ""), ("OK", "value")]);
        let out = generate_activation(&p, ShellKind::Bash);
        assert!(!out.contains("EMPTY"), "empty vars should be skipped");
        assert!(out.contains("OK=value"));
    }

    #[test]
    fn sh_quote_no_specials() {
        assert_eq!(sh_quote("/usr/local/bin"), "/usr/local/bin");
    }

    #[test]
    fn sh_quote_with_spaces() {
        assert_eq!(sh_quote("hello world"), "'hello world'");
    }

    #[test]
    fn sh_quote_with_single_quote() {
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn ps_quote_with_single_quote() {
        assert_eq!(ps_quote("it's"), "'it''s'");
    }
}
