//! Dependency resolver for loadout.
//!
//! Reads only `dep.*` fields from the Feature Index, builds a dependency DAG,
//! performs cycle detection, and returns a topologically sorted `ResolvedFeatureOrder`.
//!
//! The resolver is a **pure function**: it does not read files, modify state, or call backends.
//!
//! See: `docs/specs/algorithms/resolver.md`

use model::{CanonicalFeatureId, FeatureIndex, ResolvedFeatureOrder};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Errors produced by the resolver.
#[derive(Debug, Error, PartialEq)]
pub enum ResolverError {
    /// A feature listed in the desired set is not present in the Feature Index.
    ///
    /// Example: Profile requests `"foo/bar"` but no such feature.yaml exists.
    #[error("feature '{id}' is not present in the Feature Index")]
    FeatureNotFound { id: String },

    /// An explicit `dep.depends` entry points to a feature not in the desired set.
    ///
    /// Example: A depends on B, but B was not requested and has no transitive requirer.
    #[error("feature '{dependent}' depends on '{dependency}', but '{dependency}' is not in the desired set")]
    MissingDependency {
        dependent: String,
        dependency: String,
    },

    /// A `dep.requires` capability has no provider in the desired set.
    ///
    /// Example: Feature requires `capability:package-manager` but no brew/apt is in the profile.
    #[error("feature '{requirer}' requires capability '{capability}', but no provider is in the desired set")]
    MissingCapabilityProvider {
        requirer: String,
        capability: String,
    },

    /// A dependency cycle was detected; install order cannot be determined.
    ///
    /// Example: A depends on B, B depends on C, C depends on A.
    #[error("dependency cycle detected: {cycle}")]
    Cycle { cycle: String },
}

/// Resolve the dependency order for `desired_features` using `feature_index`.
///
/// This is a **pure function**: it does not perform I/O, does not mutate global state,
/// and is deterministic (same inputs always produce the same output).
///
/// Returns a [`ResolvedFeatureOrder`] (topologically sorted, dependencies before dependents).
///
/// # Algorithm
///
/// 1. Expand desired features to include transitive dependencies (walk `dep.depends`)
/// 2. Resolve capability-based dependencies (map `dep.requires` to `dep.provides`)
/// 3. Build dependency graph (directed edges from dependents to dependencies)
/// 4. Perform topological sort with cycle detection
///
/// See `docs/specs/algorithms/resolver.md` for full specification.
///
/// # Errors
///
/// Returns [`ResolverError`] if:
/// - a desired feature is not in the index
/// - an explicit dependency is not in the desired set
/// - a required capability has no provider in the desired set
/// - the dependency graph contains a cycle
pub fn resolve(
    feature_index: &FeatureIndex,
    desired_features: &[CanonicalFeatureId],
) -> Result<ResolvedFeatureOrder, ResolverError> {
    // Build a set for fast membership checks.
    let desired_set: HashSet<&str> = desired_features.iter().map(|id| id.as_str()).collect();

    // Validate all desired features exist in the index.
    for id in desired_features {
        if !feature_index.features.contains_key(id.as_str()) {
            return Err(ResolverError::FeatureNotFound {
                id: id.as_str().into(),
            });
        }
    }

    // Build a capability → providers map over desired features only.
    let mut capability_providers: HashMap<&str, Vec<&str>> = HashMap::new();
    for id in desired_features {
        if let Some(meta) = feature_index.features.get(id.as_str()) {
            for cap in &meta.dep.provides {
                capability_providers
                    .entry(cap.name.as_str())
                    .or_default()
                    .push(id.as_str());
            }
        }
    }

    // Build adjacency list: feature → its dependencies (within desired set).
    // Keys are feature id strings; values are sets of dependency id strings.
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for id in desired_features {
        let meta = feature_index.features.get(id.as_str()).unwrap(); // validated above
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

        // Capability-based `dep.requires` → implicit dependency on providers.
        for req in &meta.dep.requires {
            match capability_providers.get(req.name.as_str()) {
                None => {
                    return Err(ResolverError::MissingCapabilityProvider {
                        requirer: id.as_str().into(),
                        capability: req.name.clone(),
                    });
                }
                Some(providers) if providers.is_empty() => {
                    return Err(ResolverError::MissingCapabilityProvider {
                        requirer: id.as_str().into(),
                        capability: req.name.clone(),
                    });
                }
                Some(providers) => {
                    for &provider in providers {
                        if provider != id.as_str() {
                            deps.push(provider);
                        }
                    }
                }
            }
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

    let mut color: HashMap<&str, Color> = desired_features
        .iter()
        .map(|id| (id.as_str(), Color::White))
        .collect();

    let mut result: Vec<&str> = Vec::with_capacity(desired_features.len());

    // Process nodes in a deterministic order (sorted by id string).
    let mut sorted_ids: Vec<&str> = desired_features.iter().map(|id| id.as_str()).collect();
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

    // Convert to CanonicalFeatureId. All strings are known-valid at this point.
    let order = result
        .into_iter()
        .map(|s| CanonicalFeatureId::new(s).expect("resolver output id is always canonical"))
        .collect();

    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::feature_index::{CapabilityRef, DepSpec, FeatureIndex, FeatureMeta, FeatureMode};

    // Type alias to avoid clippy::type_complexity in the test helper signature.
    // (id, depends, requires, provides)
    type IndexEntry<'a> = (&'a str, &'a [&'a str], &'a [&'a str], &'a [&'a str]);

    fn make_index(entries: &[IndexEntry<'_>]) -> FeatureIndex {
        // entries: (id, depends, requires, provides)
        let features = entries
            .iter()
            .map(|&(id, depends, requires, provides)| {
                let meta = FeatureMeta {
                    spec_version: 1,
                    mode: FeatureMode::Script,
                    description: None,
                    source_dir: format!("/features/{id}"),
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
                };
                (id.to_string(), meta)
            })
            .collect();
        FeatureIndex {
            schema_version: 1,
            features,
        }
    }

    fn ids(raw: &[&str]) -> Vec<CanonicalFeatureId> {
        raw.iter()
            .map(|s| CanonicalFeatureId::new(*s).unwrap())
            .collect()
    }

    fn as_strs(order: &ResolvedFeatureOrder) -> Vec<&str> {
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
    fn missing_capability_provider() {
        let index = make_index(&[("core/git", &[], &["package_manager"], &[])]);
        let desired = ids(&["core/git"]); // no provider for package_manager
        let err = resolve(&index, &desired).unwrap_err();
        assert!(matches!(
            err,
            ResolverError::MissingCapabilityProvider { .. }
        ));
    }

    #[test]
    fn feature_not_in_index() {
        let index = make_index(&[]);
        let desired = ids(&["core/ghost"]);
        let err = resolve(&index, &desired).unwrap_err();
        assert!(matches!(err, ResolverError::FeatureNotFound { .. }));
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
