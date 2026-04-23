//! Dependency resolver for loadout.
//!
//! Reads only `dep.*` fields from the Component Index, builds a dependency DAG,
//! performs cycle detection, and returns a topologically sorted `ResolvedComponentOrder`.
//!
//! The resolver is a **pure function**: it does not read files, modify state, or call backends.
//!
//! See: `docs/specs/algorithms/resolver.md`

use model::{CanonicalComponentId, ComponentIndex, ResolvedComponentOrder};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Errors produced by the resolver.
#[derive(Debug, Error, PartialEq)]
pub enum ResolverError {
    /// A component listed in the desired set is not present in the Component Index.
    ///
    /// Example: Profile requests `"foo/bar"` but no such component.yaml exists.
    #[error("component '{id}' is not present in the Component Index")]
    ComponentNotFound { id: String },

    /// An explicit `dep.depends` entry points to a component not in the desired set.
    ///
    /// Example: A depends on B, but B was not requested and has no transitive requirer.
    #[error("component '{dependent}' depends on '{dependency}', but '{dependency}' is not in the desired set")]
    MissingDependency {
        dependent: String,
        dependency: String,
    },

    /// A dependency cycle was detected; install order cannot be determined.
    ///
    /// Example: A depends on B, B depends on C, C depends on A.
    #[error("dependency cycle detected: {cycle}")]
    Cycle { cycle: String },
}

/// Resolve the dependency order for `desired_components` using `component_index`.
///
/// This is a **pure function**: it does not perform I/O, does not mutate global state,
/// and is deterministic (same inputs always produce the same output).
///
/// Returns a [`ResolvedComponentOrder`] (topologically sorted, dependencies before dependents).
///
/// # Algorithm
///
/// 1. Expand desired components to include transitive dependencies (walk `dep.depends`)
/// 2. Resolve capability-based dependencies (map `dep.requires` to `dep.provides`)
/// 3. Build dependency graph (directed edges from dependents to dependencies)
/// 4. Perform topological sort with cycle detection
///
/// See `docs/specs/algorithms/resolver.md` for full specification.
///
/// # Errors
///
/// Returns [`ResolverError`] if:
/// - a desired component is not in the index
/// - an explicit dependency is not in the desired set
/// - the dependency graph contains a cycle
///
/// `dep.requires` with no provider in the desired set is **not** an error; the ordering
/// constraint is simply omitted (the backend may already be installed externally).
pub fn resolve(
    component_index: &ComponentIndex,
    desired_components: &[CanonicalComponentId],
) -> Result<ResolvedComponentOrder, ResolverError> {
    // Build a set for fast membership checks.
    let desired_set: HashSet<&str> = desired_components.iter().map(|id| id.as_str()).collect();

    // Validate all desired components exist in the index.
    for id in desired_components {
        if !component_index.components.contains_key(id.as_str()) {
            return Err(ResolverError::ComponentNotFound {
                id: id.as_str().into(),
            });
        }
    }

    // Build a capability → providers map over desired components only.
    let mut capability_providers: HashMap<&str, Vec<&str>> = HashMap::new();
    for id in desired_components {
        if let Some(meta) = component_index.components.get(id.as_str()) {
            for cap in &meta.dep.provides {
                capability_providers
                    .entry(cap.name.as_str())
                    .or_default()
                    .push(id.as_str());
            }
        }
    }

    // Build adjacency list: component → its dependencies (within desired set).
    // Keys are component id strings; values are sets of dependency id strings.
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for id in desired_components {
        let meta = component_index.components.get(id.as_str()).unwrap(); // validated above
        let mut deps: Vec<&str> = Vec::new();

        // Explicit `dep.depends` entries.
        for dep_id in &meta.dep.depends {
            if !desired_set.contains(dep_id.as_str()) {
                return Err(ResolverError::MissingDependency {
                    dependent: id.as_str().into(),
                    dependency: dep_id.clone(),
                });
            }
            deps.push(dep_id.as_str());
        }

        // Capability-based `dep.requires` → implicit ordering dependency on providers.
        // Soft: if no provider is present in the desired set, the constraint is silently skipped.
        // The required capability may be satisfied by an externally installed tool.
        for req in &meta.dep.requires {
            if let Some(providers) = capability_providers.get(req.name.as_str()) {
                for &provider in providers {
                    if provider != id.as_str() {
                        deps.push(provider);
                    }
                }
            }
            // No provider in desired set → no ordering edge (not an error).
        }

        // Deduplicate while preserving first-seen order (for determinism).
        deps.dedup();
        adj.insert(id.as_str(), deps);
    }

    // Topological sort via iterative DFS with cycle detection.
    // We use an explicit stack to avoid recursion depth issues for large graphs.
    //
    // State per node:
    //   White (not visited) → Gray (in current DFS path) → Black (fully processed)
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    let mut color: HashMap<&str, Color> = desired_components
        .iter()
        .map(|id| (id.as_str(), Color::White))
        .collect();

    let mut result: Vec<&str> = Vec::with_capacity(desired_components.len());

    // Process nodes in a deterministic order (sorted by id string).
    let mut sorted_ids: Vec<&str> = desired_components.iter().map(|id| id.as_str()).collect();
    sorted_ids.sort_unstable();

    for start in sorted_ids {
        if color[start] == Color::Black {
            continue;
        }

        // Iterative DFS. Stack items: (node, index into its adjacency list, DFS path for error).
        let mut stack: Vec<(&str, usize, Vec<String>)> = Vec::new();
        let path = vec![start.to_string()];
        color.insert(start, Color::Gray);
        stack.push((start, 0, path));

        while let Some((node, next_dep_idx, path)) = stack.last_mut() {
            let node = *node;
            let deps = &adj[node];

            if *next_dep_idx < deps.len() {
                let dep = deps[*next_dep_idx];
                *next_dep_idx += 1;

                match color[dep] {
                    Color::Black => {} // already processed, skip
                    Color::Gray => {
                        // Found a back-edge: cycle detected.
                        let mut cycle_path = path.clone();
                        cycle_path.push(dep.to_string());
                        return Err(ResolverError::Cycle {
                            cycle: cycle_path.join(" → "),
                        });
                    }
                    Color::White => {
                        color.insert(dep, Color::Gray);
                        let mut new_path = path.clone();
                        new_path.push(dep.to_string());
                        stack.push((dep, 0, new_path));
                    }
                }
            } else {
                // All dependencies of `node` have been processed → emit it.
                color.insert(node, Color::Black);
                result.push(node);
                stack.pop();
            }
        }
    }

    // Convert to CanonicalComponentId. All strings are known-valid at this point.
    let order = result
        .into_iter()
        .map(|s| CanonicalComponentId::new(s).expect("resolver output id is always canonical"))
        .collect();

    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::component_index::{
        CapabilityRef, ComponentIndex, ComponentMeta, ComponentMode, DepSpec,
    };

    // Type alias to avoid clippy::type_complexity in the test helper signature.
    // (id, depends, requires, provides)
    type IndexEntry<'a> = (&'a str, &'a [&'a str], &'a [&'a str], &'a [&'a str]);

    fn make_index(entries: &[IndexEntry<'_>]) -> ComponentIndex {
        // entries: (id, depends, requires, provides)
        let components = entries
            .iter()
            .map(|&(id, depends, requires, provides)| {
                let meta = ComponentMeta {
                    spec_version: 1,
                    mode: ComponentMode::Script,
                    description: None,
                    source_dir: format!("/components/{id}"),
                    dep: DepSpec {
                        depends: depends.iter().map(|s| s.to_string()).collect(),
                        requires: requires
                            .iter()
                            .map(|s| CapabilityRef {
                                name: s.to_string(),
                            })
                            .collect(),
                        provides: provides
                            .iter()
                            .map(|s| CapabilityRef {
                                name: s.to_string(),
                            })
                            .collect(),
                    },
                    spec: None,
                    scripts: None,
                    params_schema: None,
                };
                (id.to_string(), meta)
            })
            .collect();
        ComponentIndex {
            schema_version: 1,
            components,
        }
    }

    fn ids(raw: &[&str]) -> Vec<CanonicalComponentId> {
        raw.iter()
            .map(|s| CanonicalComponentId::new(*s).unwrap())
            .collect()
    }

    fn as_strs(order: &ResolvedComponentOrder) -> Vec<&str> {
        order.iter().map(|id| id.as_str()).collect()
    }

    #[test]
    fn no_deps() {
        let index = make_index(&[("core/bash", &[], &[], &[]), ("core/git", &[], &[], &[])]);
        let desired = ids(&["core/bash", "core/git"]);
        let order = resolve(&index, &desired).unwrap();
        // Both are present; order is deterministic (alphabetical when no deps).
        assert_eq!(order.len(), 2);
        let strs = as_strs(&order);
        assert!(strs.contains(&"core/bash"));
        assert!(strs.contains(&"core/git"));
    }

    #[test]
    fn simple_dependency() {
        // neovim depends on git
        let index = make_index(&[
            ("core/git", &[], &[], &[]),
            ("core/neovim", &["core/git"], &[], &[]),
        ]);
        let desired = ids(&["core/neovim", "core/git"]);
        let order = resolve(&index, &desired).unwrap();
        let strs = as_strs(&order);
        let git_pos = strs.iter().position(|&s| s == "core/git").unwrap();
        let nvim_pos = strs.iter().position(|&s| s == "core/neovim").unwrap();
        assert!(git_pos < nvim_pos, "git must come before neovim");
    }

    #[test]
    fn capability_based_dependency() {
        // brew provides package_manager; git requires package_manager
        let index = make_index(&[
            ("core/brew", &[], &[], &["package_manager"]),
            ("core/git", &[], &["package_manager"], &[]),
        ]);
        let desired = ids(&["core/brew", "core/git"]);
        let order = resolve(&index, &desired).unwrap();
        let strs = as_strs(&order);
        let brew_pos = strs.iter().position(|&s| s == "core/brew").unwrap();
        let git_pos = strs.iter().position(|&s| s == "core/git").unwrap();
        assert!(
            brew_pos < git_pos,
            "brew (provider) must come before git (requirer)"
        );
    }

    #[test]
    fn diamond_dependency() {
        // A depends on B and C; B and C both depend on D
        let index = make_index(&[
            ("core/d", &[], &[], &[]),
            ("core/b", &["core/d"], &[], &[]),
            ("core/c", &["core/d"], &[], &[]),
            ("core/a", &["core/b", "core/c"], &[], &[]),
        ]);
        let desired = ids(&["core/a", "core/b", "core/c", "core/d"]);
        let order = resolve(&index, &desired).unwrap();
        let strs = as_strs(&order);
        let pos = |s| strs.iter().position(|&x| x == s).unwrap();
        assert!(pos("core/d") < pos("core/b"));
        assert!(pos("core/d") < pos("core/c"));
        assert!(pos("core/b") < pos("core/a"));
        assert!(pos("core/c") < pos("core/a"));
    }

    #[test]
    fn cycle_detected() {
        // a → b → a
        let index = make_index(&[
            ("core/a", &["core/b"], &[], &[]),
            ("core/b", &["core/a"], &[], &[]),
        ]);
        let desired = ids(&["core/a", "core/b"]);
        let err = resolve(&index, &desired).unwrap_err();
        assert!(matches!(err, ResolverError::Cycle { .. }));
    }

    #[test]
    fn missing_dependency() {
        // neovim depends on git, but git is not in desired set
        let index = make_index(&[
            ("core/git", &[], &[], &[]),
            ("core/neovim", &["core/git"], &[], &[]),
        ]);
        let desired = ids(&["core/neovim"]); // git missing
        let err = resolve(&index, &desired).unwrap_err();
        assert!(matches!(err, ResolverError::MissingDependency { .. }));
    }

    #[test]
    fn requires_without_provider_is_soft() {
        // No provider for package_manager in desired set → not an error,
        // ordering constraint is simply omitted.
        let index = make_index(&[("core/git", &[], &["package_manager"], &[])]);
        let desired = ids(&["core/git"]);
        let order = resolve(&index, &desired).unwrap();
        assert_eq!(order.len(), 1);
        assert_eq!(order[0].as_str(), "core/git");
    }

    #[test]
    fn component_not_in_index() {
        let index = make_index(&[]);
        let desired = ids(&["core/ghost"]);
        let err = resolve(&index, &desired).unwrap_err();
        assert!(matches!(err, ResolverError::ComponentNotFound { .. }));
    }

    #[test]
    fn deterministic_output() {
        let index = make_index(&[
            ("core/a", &[], &[], &[]),
            ("core/b", &[], &[], &[]),
            ("core/c", &[], &[], &[]),
        ]);
        let desired = ids(&["core/c", "core/a", "core/b"]);
        let order1 = resolve(&index, &desired).unwrap();
        let order2 = resolve(&index, &desired).unwrap();
        assert_eq!(order1, order2);
    }
}
