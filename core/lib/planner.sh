#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: planner
#
# Responsibility:
#   PURE decision engine. Converts (desired_resource_graph, state) into a
#   structured plan object describing what the executor should do.
#   Never executes anything.
#
# Public API:
#   planner_run <rrg_json> <sorted_features_nameref> <profile_file>  → plan JSON (stdout)
#
# Internal phases (each PURE function):
#   _planner_diff       rrg × state → diff JSON array (includes profile_version per feature)
#   _planner_classify   diff JSON   → classified JSON array
#   _planner_decide     classified JSON × sorted order → plan JSON
#
# Inputs:
#   rrg_json              — ResolvedResourceGraph JSON (DRG + desired_backend, from PolicyResolver)
#   sorted_features       — topologically sorted canonical feature IDs from Resolver
#   profile_file          — path to profile YAML; used to read version hints per feature
#   _STATE_JSON (global)  — current authoritative state (read-only)
#
# Planner receives policy-resolved resources (desired_backend in RRG) and reads
# profile version hints directly; it does NOT receive raw policy objects.
#
# Plan JSON schema:
#   {
#     "actions": [
#       {"feature": "core/git",  "operation": "create",   "details": {}},
#       {"feature": "core/node", "operation": "replace",  "details": {}},
#       {"feature": "core/tmux", "operation": "strengthen",
#        "details": {"add_resources": [{"kind": "fs", "id": "fs:tmux.conf"}]}}
#     ],
#     "noops":   [{"feature": "core/bash"}],
#     "blocked": [{"feature": "user/legacy", "reason": "unknown resource kind: registry"}],
#     "summary": {"create": 1, "destroy": 0, "replace": 1, "replace_backend": 0,
#                 "strengthen": 1, "noop": 1, "blocked": 0}
#   }
#
# Classification table:
#   in_desired=false, in_state=true                              → destroy
#   in_desired=true,  in_state=false                             → create
#   in_desired=true,  in_state=true, desired_resources empty     → noop  (script feature)
#   in_desired=true,  in_state=true, incompatible resource change → replace
#   in_desired=true,  in_state=true, backend mismatch only       → replace_backend
#   in_desired=true,  in_state=true, strict superset + compat    → strengthen
#   in_desired=true,  in_state=true, identical resources         → noop
#   unknown resource kind in desired or state                    → blocked
# -----------------------------------------------------------------------------

# This module reads _STATE_JSON as a READ-ONLY input.
# _PLANNER_PROFILE_FILE is set by planner_run and read by _planner_diff.
_PLANNER_PROFILE_FILE=""

# Valid resource kinds; anything else causes a "blocked" classification.
# Guard re-declaration when this file is re-sourced.
[[ "${_PLANNER_VALID_KINDS_SET:-}" == "1" ]] || {
    readonly _PLANNER_VALID_KINDS="package runtime fs"
    readonly _PLANNER_VALID_KINDS_SET="1"
}

# ── Profile helpers ──────────────────────────────────────────────────────────

# _planner_profile_version <profile_file> <canonical_id>
# Extract the version hint for a feature from a profile file.
# Tries canonical ID first (e.g. "core/node"), then bare name (e.g. "node").
# Prints the version string, or empty string if not found.
_planner_profile_version() {
    local profile_file="$1"
    local canonical_id="$2"

    [[ -z "$profile_file" || ! -f "$profile_file" ]] && echo "" && return 0

    # Try canonical ID bracket notation (handles "/" in key)
    local ver
    ver=$(yq eval ".features[\"${canonical_id}\"].version // \"\"" "$profile_file" 2>/dev/null)
    if [[ -n "$ver" && "$ver" != "null" ]]; then echo "$ver"; return 0; fi

    # Bare name fallback for profiles without source_id prefix
    local bare="${canonical_id#*/}"
    if [[ "$bare" != "$canonical_id" ]]; then
        ver=$(yq eval ".features[\"${bare}\"].version // \"\"" "$profile_file" 2>/dev/null)
        if [[ -n "$ver" && "$ver" != "null" ]]; then echo "$ver"; return 0; fi
    fi

    echo ""
}

# ── State helpers (read-only) ─────────────────────────────────────────────────

# _planner_state_has_unknown_kind <feature>
# Return 0 if the feature has any resource with an unrecognised kind in state.
_planner_state_has_unknown_kind() {
    local feature="$1"
    local count
    count=$(printf '%s' "$_STATE_JSON" | jq --arg f "$feature" \
        '.features[$f].resources // [] | map(select(.kind | IN("package","runtime","fs") | not)) | length')
    [[ "$count" -gt 0 ]]
}

# _planner_state_unknown_kinds_csv <feature>
# Print comma-separated list of unrecognised resource kinds from state (for error messages).
_planner_state_unknown_kinds_csv() {
    local feature="$1"
    printf '%s' "$_STATE_JSON" | jq -r --arg f "$feature" \
        '.features[$f].resources // [] | map(select(.kind | IN("package","runtime","fs") | not)) | map(.kind) | unique | join(", ")'
}

# ── Phase 1: Diff ─────────────────────────────────────────────────────────────

# _planner_diff <rrg_json> <sorted_features_nameref>
# Compare desired features (ResolvedResourceGraph) against current state.
# Outputs a JSON array of diff objects to stdout.
#
# Diff object schema:
#   {
#     "feature":                string,
#     "in_desired":             bool,
#     "in_state":               bool,
#     "desired_resource_count": number,
#     "desired_resources":      array,   // resources from RRG (with desired_backend)
#     "state_resources":        array,   // resources from state ([] if not installed)
#     "has_blocked_resources":  bool,
#     "blocked_reason":         string | null,
#     "profile_version":        string    // version hint from profile (empty if not set)
#   }
_planner_diff() {
    local drg_json="$1"
    local -n _diff_sorted="$2"

    local diff_json="[]"

    # ── Desired features (in sorted dependency order) ──
    for feature in "${_diff_sorted[@]}"; do
        local in_state="false"
        state_has_feature "$feature" && in_state="true"

        # Extract desired resources from RRG
        local desired_resources desired_count
        desired_resources=$(printf '%s' "$drg_json" \
            | jq -c --arg f "$feature" '.features[$f].resources // []')
        desired_count=$(printf '%s' "$desired_resources" | jq 'length')

        # Extract state resources
        local state_resources="[]"
        if [[ "$in_state" == "true" ]]; then
            state_resources=$(printf '%s' "$_STATE_JSON" \
                | jq -c --arg f "$feature" '.features[$f].resources // []')
        fi

        # Check for unknown resource kinds
        local has_blocked="false"
        local blocked_reason="null"

        # Unknown kind in desired resources
        local unk_desired
        unk_desired=$(printf '%s' "$desired_resources" | jq -r \
            '[.[] | select(.kind | IN("package","runtime","fs") | not) | .kind] | unique | join(", ")')
        if [[ -n "$unk_desired" ]]; then
            has_blocked="true"
            blocked_reason="\"unknown resource kind: $unk_desired\""
        fi

        # Unknown kind in state resources
        if [[ "$in_state" == "true" && "$has_blocked" == "false" ]]; then
            local unk_state
            unk_state=$(printf '%s' "$state_resources" | jq -r \
                '[.[] | select(.kind | IN("package","runtime","fs") | not) | .kind] | unique | join(", ")')
            if [[ -n "$unk_state" ]]; then
                has_blocked="true"
                blocked_reason="\"unknown resource kind in state: $unk_state\""
            fi
        fi

        # Read version hint for this feature from profile (used for runtime version comparison)
        local profile_version=""
        if [[ -n "${_PLANNER_PROFILE_FILE:-}" ]]; then
            profile_version=$(_planner_profile_version "$_PLANNER_PROFILE_FILE" "$feature")
        fi

        diff_json=$(printf '%s' "$diff_json" | jq \
            --arg  f          "$feature" \
            --argjson in_state  "$in_state" \
            --argjson desired   "$desired_resources" \
            --argjson state     "$state_resources" \
            --argjson hb        "$has_blocked" \
            --argjson br        "$blocked_reason" \
            --argjson dc        "$desired_count" \
            --arg pv            "$profile_version" \
            '. + [{
                feature:                $f,
                in_desired:             true,
                in_state:               $in_state,
                desired_resource_count: $dc,
                desired_resources:      $desired,
                state_resources:        $state,
                has_blocked_resources:  $hb,
                blocked_reason:         $br,
                profile_version:        $pv
            }]')
    done

    # ── Installed features not in desired (candidates for destroy) ──
    local -a installed_features
    mapfile -t installed_features < <(printf '%s' "$_STATE_JSON" | jq -r '.features | keys[]')

    for feature in "${installed_features[@]}"; do
        local covered=false
        local desired
        for desired in "${_diff_sorted[@]}"; do
            [[ "$desired" == "$feature" ]] && covered=true && break
        done
        [[ "$covered" == "true" ]] && continue

        diff_json=$(printf '%s' "$diff_json" | jq \
            --arg f "$feature" \
            '. + [{
                feature:                $f,
                in_desired:             false,
                in_state:               true,
                desired_resource_count: 0,
                desired_resources:      [],
                state_resources:        [],
                has_blocked_resources:  false,
                blocked_reason:         null
            }]')
    done

    printf '%s' "$diff_json"
}

# ── Phase 2: Classification ───────────────────────────────────────────────────

# _planner_classify <diff_json>
# Apply the decision table to each diff entry.
# Outputs a JSON array of classified objects to stdout.
#
# Classified object schema:
#   {feature, classification, reason?, add_resources?}
#
# Resource semantic key matching:
#   Desired: pkg:<name>  rt:<name>  fs:<basename(target or path)>
#   State:   pkg:<package.name>  rt:<runtime.name>  fs:<basename(fs.path)>
_planner_classify() {
    local diff_json="$1"

    printf '%s' "$diff_json" | jq '[.[] |
        if .has_blocked_resources then
            {feature: .feature, classification: "blocked", reason: (.blocked_reason // "unknown resource kind")}

        elif (.in_desired and (.in_state | not)) then
            {feature: .feature, classification: "create"}

        elif ((.in_desired | not) and .in_state) then
            {feature: .feature, classification: "destroy"}

        elif (.in_desired and .in_state) then
            (.desired_resource_count) as $dc |
            (.profile_version // "") as $pv |
            if $dc == 0 then
                # Script feature (empty desired resources): classify by feature presence only
                {feature: .feature, classification: "noop"}
            else
                (.desired_resources) as $desired |
                (.state_resources)   as $state   |

                # Build keyed arrays for semantic matching
                ($desired | map({
                    key: (
                        if   .kind == "package" then "pkg:" + (.name // "?")
                        elif .kind == "runtime" then "rt:"  + (.name // "?")
                        elif .kind == "fs"      then "fs:"  + ((.target // .path // "") | split("/") | last)
                        else "other:" + .kind
                        end
                    ),
                    res: .
                })) as $d_keyed |

                ($state | map({
                    key: (
                        if   .kind == "package" then "pkg:" + (.package.name // "?")
                        elif .kind == "runtime" then "rt:"  + (.runtime.name // "?")
                        elif .kind == "fs"      then "fs:"  + (.fs.path | split("/") | last)
                        else "other:" + .kind
                        end
                    ),
                    res: .
                })) as $s_keyed |

                ($d_keyed | map(.key) | sort) as $dk |
                ($s_keyed | map(.key) | sort) as $sk |

                # Set operations
                ($dk | map(. as $k | select($sk | contains([$k]))))             as $common |
                ($dk | map(. as $k | select($sk | contains([$k]) | not)))       as $d_only  |
                ($sk | map(. as $k | select($dk | contains([$k]) | not)))       as $s_only  |

                # Compatibility of common resources
                ($common | map(
                    . as $k |
                    ($d_keyed | map(select(.key == $k)) | first | .res) as $dr |
                    ($s_keyed | map(select(.key == $k)) | first | .res) as $sr |
                    if $dr.kind == "package" then
                        if ($dr.desired_backend // "?") != ($sr.backend // "?") then "backend_mismatch"
                        else "compatible"
                        end
                    elif $dr.kind == "runtime" then
                        if ($dr.desired_backend // "?") != ($sr.backend // "?") then "backend_mismatch"
                        elif ($pv | length) > 0 and ($pv != ($sr.runtime.version // "")) then "version_mismatch"
                        else "compatible"
                        end
                    elif $dr.kind == "fs" then
                        if (($dr.target // $dr.path // "") != ($sr.fs.path // "")) then "incompatible"
                        elif (($dr.entry_type // "") | length) > 0 and ($dr.entry_type != ($sr.fs.entry_type // "")) then "incompatible"
                        elif ($dr.op // "link") != ($sr.fs.op // "link") then "incompatible"
                        else "compatible"
                        end
                    else "compatible"
                    end
                )) as $compat |

                (($compat | map(select(. == "backend_mismatch")) | length) > 0) as $has_bm  |
                (($compat | map(select(. != "compatible" and . != "backend_mismatch")) | length) > 0) as $has_inc |

                if $has_inc or ($s_only | length) > 0 then
                    # Incompatible mutation or state has resources not in desired
                    {feature: .feature, classification: "replace"}
                elif $has_bm then
                    {feature: .feature, classification: "replace_backend"}
                elif ($d_only | length) > 0 then
                    # All state resources in desired, all common compatible, desired has extras
                    {
                        feature:        .feature,
                        classification: "strengthen",
                        add_resources:  ($d_only | map(
                            . as $k |
                            ($d_keyed | map(select(.key == $k)) | first | .res) |
                            {kind: .kind, id: (.id // .kind)}
                        ))
                    }
                else
                    {feature: .feature, classification: "noop"}
                end
            end
        else
            {feature: .feature, classification: "noop"}
        end
    ]'
}

# ── Phase 3: Decision ─────────────────────────────────────────────────────────

# _planner_decide <classified_json>
# Apply ordering rules and produce the final plan JSON.
# Ordering:
#   1. destroy         – reverse order
#   2. replace         – dependency order
#   3. replace_backend – dependency order (treated as replace)
#   4. strengthen      – dependency order (only adds resources)
#   5. create          – dependency order
_planner_decide() {
    local classified_json="$1"

    printf '%s' "$classified_json" | jq '
        . as $items |

        ($items | map(select(.classification == "destroy"))         | reverse) as $destroys         |
        ($items | map(select(.classification == "replace")))                   as $replaces         |
        ($items | map(select(.classification == "replace_backend")))           as $replace_backends |
        ($items | map(select(.classification == "strengthen")))                as $strengthens      |
        ($items | map(select(.classification == "create")))                    as $creates          |
        ($items | map(select(.classification == "blocked")))                   as $blocked          |
        ($items | map(select(.classification == "noop")))                      as $noops            |

        (
            [ $destroys[]         | {feature: .feature, operation: "destroy",         details: {}} ] +
            [ $replaces[]         | {feature: .feature, operation: "replace",         details: {}} ] +
            [ $replace_backends[] | {feature: .feature, operation: "replace_backend", details: {}} ] +
            [ $strengthens[]      | {feature: .feature, operation: "strengthen",
                                     details: {add_resources: (.add_resources // [])}} ] +
            [ $creates[]          | {feature: .feature, operation: "create",          details: {}} ]
        ) as $actions |

        {
            actions: $actions,
            noops:   ($noops   | map({feature: .feature})),
            blocked: ($blocked | map({feature: .feature, reason: (.reason // "unknown resource kind")})),
            summary: {
                create:          ($creates          | length),
                destroy:         ($destroys         | length),
                replace:         ($replaces         | length),
                replace_backend: ($replace_backends | length),
                strengthen:      ($strengthens      | length),
                noop:            ($noops            | length),
                blocked:         ($blocked          | length)
            }
        }
    '
}

# ── Public API ────────────────────────────────────────────────────────────────

# planner_run <rrg_json> <sorted_features_nameref> <profile_file>
# Full planning pipeline: diff → classify → decide.
# Outputs plan JSON to stdout.
#
# Inputs (read-only module globals):
#   _STATE_JSON — loaded state (from state_load / state_init)
planner_run() {
    local drg_json="$1"
    local sorted_features_ref="$2"
    _PLANNER_PROFILE_FILE="${3:-}"

    if [[ -z "$drg_json" ]]; then
        log_error "planner_run: drg_json is required"
        return 1
    fi

    local diff_json
    diff_json=$(_planner_diff "$drg_json" "$sorted_features_ref") || return 1

    local classified_json
    classified_json=$(_planner_classify "$diff_json") || return 1

    _planner_decide "$classified_json"
}
