# -----------------------------------------------------------------------------
# Module: executor (PowerShell)
#
# Responsibility:
#   IMPURE executor. Receives a plan JSON string from planner and executes it.
#   Manages all package/runtime/file installations and state commits.
#
# Public API:
#   Invoke-ExecutorRun <PlanJson>
#
# Execution contract:
#   - Blocked features in plan.blocked are reported and skipped.
#   - Actions are executed in plan.actions order (destroy → replace → create).
#   - For each action, executor reads feature.yaml to determine packages/runtimes/files.
#   - On any failure: abort immediately with non-zero exit.
#     Partial execution is left in place; state reflects what succeeded.
#
# State commit model (Phase 4):
#   Executor owns all state writes:
#     install:  State-PatchBegin → packages/runtimes/files → State-PatchFinalize
#               → run install.ps1 (for secondary setup)
#     destroy:  _Executor-RemoveResources (fs rm + backend uninstall)
#               → run uninstall.ps1 (for secondary cleanup)
#               → State-PatchBegin → State-PatchRemoveFeature → State-PatchFinalize
#
#   Feature scripts must NOT call State-RemoveFeature, State-AddPackage,
#   Install-Package, Install-Runtime.
#
# feature.yaml package/runtime/files schema: see executor.sh for full docs.
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This module expects env.ps1, logger.ps1, state.ps1, backend_registry.ps1 to be
# dot-sourced by the caller (orchestrator).

if (-not (Get-Command Source-Registry-GetFeatureDir -ErrorAction SilentlyContinue)) {
    . "$env:LOADOUT_ROOT\core\lib\source_registry.ps1"
}

if (-not (Get-Command Invoke-DeclarativeExecutorRun -ErrorAction SilentlyContinue)) {
    . "$env:LOADOUT_ROOT\core\lib\declarative_executor.ps1"
}

# ── Meta helpers ──────────────────────────────────────────────────────────────

# _Executor-GetFeatureMode <Feature>
# Read .mode from feature.yaml. Returns "declarative" or "script".
function _Executor-GetFeatureMode {
    param([Parameter(Mandatory = $true)] [string]$Feature)

    try {
        $metaFile = _Executor-ResolveFeature -Feature $Feature
        $mode = & yq eval '.mode' $metaFile 2>$null
        if (-not [string]::IsNullOrWhiteSpace($mode) -and $mode -ne 'null') { return $mode.Trim() }
    } catch {}
    return "declarative"
}

function _Executor-GetFeatureDir {
    param([Parameter(Mandatory=$true)] [string]$Feature)

    $parts = Canonical-Id-Parse $Feature
    $featureRoot = Source-Registry-GetFeatureDir -SourceId $parts.SourceId
    return Join-Path $featureRoot $parts.Name
}

# _Executor-ResolveFeature <Feature>
# Return base feature.yaml path. Throws if not found.
function _Executor-ResolveFeature {
    param([Parameter(Mandatory=$true)] [string]$Feature)

    $featureDir = _Executor-GetFeatureDir -Feature $Feature
    $meta = Join-Path $featureDir "feature.yaml"
    if (-not (Test-Path $meta)) {
        Log-Error "_Executor-ResolveFeature: feature.yaml not found for: $Feature"
        throw "feature.yaml not found: $meta"
    }
    return $meta
}

# _Executor-ResolvePlatformFeature <Feature>
# Return platform-specific feature.yaml path, or $null if none exists.
function _Executor-ResolvePlatformFeature {
    param([Parameter(Mandatory=$true)] [string]$Feature)

    $dir = _Executor-GetFeatureDir -Feature $Feature

    switch ($global:LOADOUT_PLATFORM) {
        "windows" {
            $p = Join-Path $dir "feature.windows.yaml"
            if (Test-Path $p) { return $p }
        }
        { $_ -in @("linux", "wsl") } {
            $p = Join-Path $dir "feature.linux.yaml"
            if (Test-Path $p) { return $p }
        }
    }
    return $null
}

# _Executor-GetPkgsFromFeature <MetaFile>
# Return string[] of package names from a feature.yaml file.
# Supports string form ("tmux") and mapping form ({name: tmux, managed: false}).
function _Executor-GetPkgsFromFeature {
    param([string]$MetaFile)

    if ([string]::IsNullOrWhiteSpace($MetaFile) -or -not (Test-Path $MetaFile)) {
        return @()
    }

    $raw = & yq eval '.packages // [] | .[] | .name // .' $MetaFile 2>$null
    if ($LASTEXITCODE -ne 0 -or $null -eq $raw) { return @() }

    return @($raw | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
}

# _Executor-PkgManaged <Feature> <PkgName>
# Return $true if the package should be uninstalled on feature remove; $false if managed:false.
function _Executor-PkgManaged {
    param(
        [Parameter(Mandatory=$true)] [string]$Feature,
        [Parameter(Mandatory=$true)] [string]$PkgName
    )

    try { $metaFile = _Executor-ResolveFeature -Feature $Feature } catch { return $true }
    $platformMeta = _Executor-ResolvePlatformFeature -Feature $Feature

    foreach ($f in @($metaFile, $platformMeta)) {
        if ([string]::IsNullOrWhiteSpace($f) -or -not (Test-Path $f)) { continue }

        # yq v4: select the entry matching the package name (string or map), read .managed
        # Use env(YQ_PKG) to avoid PS 5.1 double-quote stripping on native exe args
        $env:YQ_PKG = $PkgName
        $flag = & yq eval '.packages // [] | .[] | select((.name // .) == env(YQ_PKG)) | .managed' $f 2>$null |
                    Select-Object -First 1
        if ($flag -eq "false") { return $false }
    }
    return $true
}

# _Executor-GetRuntimesJson <MetaFile>
# Return PSCustomObject[] of runtime entries from a feature.yaml file.
function _Executor-GetRuntimesJson {
    param([string]$MetaFile)

    if ([string]::IsNullOrWhiteSpace($MetaFile) -or -not (Test-Path $MetaFile)) {
        return @()
    }

    $json = & yq eval -o=json '.runtimes // []' $MetaFile 2>$null
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($json)) { return @() }

    return @($json | ConvertFrom-Json)
}

# _Executor-GetFilesJson <MetaFile>
# Return PSCustomObject[] of file entry objects from a feature.yaml file.
function _Executor-GetFilesJson {
    param([string]$MetaFile)

    if ([string]::IsNullOrWhiteSpace($MetaFile) -or -not (Test-Path $MetaFile)) {
        return @()
    }

    $json = & yq eval -o=json '.files // []' $MetaFile 2>$null
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($json)) { return @() }

    return @($json | ConvertFrom-Json)
}

# ── Resource operations ───────────────────────────────────────────────────────

# _Executor-ApplyPackages <Feature> <MetaFile> [<PlatformMetaFile>]
# Install all packages declared in feature.yaml and add them to the active state patch.
# Skips packages where Backend-PackageExists returns true.
function _Executor-ApplyPackages {
    param(
        [Parameter(Mandatory=$true)]  [string]$Feature,
        [Parameter(Mandatory=$true)]  [string]$MetaFile,
        [string]$PlatformMetaFile = ""
    )

    $allPkgs  = @()
    $allPkgs += _Executor-GetPkgsFromFeature -MetaFile $MetaFile
    $allPkgs += _Executor-GetPkgsFromFeature -MetaFile $PlatformMetaFile

    # Deduplicate; guard against empty array producing blank entries
    $uniquePkgs = @($allPkgs | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Sort-Object -Unique)
    if ($uniquePkgs.Count -eq 0) { return $true }

    foreach ($pkg in $uniquePkgs) {
        $backend = Resolve-BackendFor -Kind "package" -Name $pkg
        Load-Backend -BackendId $backend

        if (Backend-Call "PackageExists" $pkg 2>$null) {
            Log-Info "    package already installed: $pkg"
        } else {
            Log-Info "    installing package: $pkg"
            Backend-Call "InstallPackage" $pkg
            if ($LASTEXITCODE -ne 0) {
                Log-Error "executor: failed to install package: $pkg"
                return $false
            }
        }

        $resource = [PSCustomObject]@{
            kind    = "package"
            id      = "pkg:$pkg"
            backend = $backend
            package = [PSCustomObject]@{ name = $pkg; version = $null }
        }
        State-PatchAddResource -Feature $Feature -ResourceObject $resource
    }
    return $true
}

# _Executor-ApplyRuntimes <Feature> <MetaFile> [<PlatformMetaFile>] [<ConfigVersion>]
# Install all runtimes declared in feature.yaml and add them to the active state patch.
# Version resolution: explicit in feature.yaml > config_version from profile > "latest"
function _Executor-ApplyRuntimes {
    param(
        [Parameter(Mandatory=$true)]  [string]$Feature,
        [Parameter(Mandatory=$true)]  [string]$MetaFile,
        [string]$PlatformMetaFile = "",
        [string]$ConfigVersion    = ""
    )

    $allRts = @()
    $allRts += _Executor-GetRuntimesJson -MetaFile $MetaFile
    $allRts += _Executor-GetRuntimesJson -MetaFile $PlatformMetaFile

    $seen      = [System.Collections.Generic.HashSet[string]]::new()
    $uniqueRts = @()
    foreach ($rt in $allRts) {
        if ($seen.Add($rt.name)) { $uniqueRts += $rt }
    }

    if ($uniqueRts.Count -eq 0) { return $true }

    foreach ($rtEntry in $uniqueRts) {
        $rtName    = $rtEntry.name
        $rtMetaVer = if ($rtEntry.PSObject.Properties['version'] -and
                        -not [string]::IsNullOrWhiteSpace($rtEntry.version)) {
                        [string]$rtEntry.version } else { "" }

        # Version resolution
        $rtVersion = if     (-not [string]::IsNullOrWhiteSpace($rtMetaVer))    { $rtMetaVer }
                     elseif (-not [string]::IsNullOrWhiteSpace($ConfigVersion)) { $ConfigVersion }
                     else   { "latest" }

        $backend = Resolve-BackendFor -Kind "runtime" -Name $rtName
        Load-Backend -BackendId $backend

        $actualVersion = $rtVersion
        if (Backend-Call "RuntimeExists" $rtName $rtVersion 2>$null) {
            Log-Info "    runtime already installed: ${rtName}@${rtVersion}"
        } else {
            Log-Info "    installing runtime: ${rtName}@${rtVersion}"
            $installedVer = Backend-Call "InstallRuntime" $rtName $rtVersion
            if ($LASTEXITCODE -ne 0) {
                Log-Error "executor: failed to install runtime: ${rtName}@${rtVersion}"
                return $false
            }
            if (-not [string]::IsNullOrWhiteSpace($installedVer)) {
                $actualVersion = [string]$installedVer
            }
        }

        $resource = [PSCustomObject]@{
            kind    = "runtime"
            id      = "rt:${rtName}@${actualVersion}"
            backend = $backend
            runtime = [PSCustomObject]@{ name = $rtName; version = $actualVersion }
        }
        State-PatchAddResource -Feature $Feature -ResourceObject $resource
    }
    return $true
}

# _Executor-ExpandPath <Path>
# Expand leading ~ to $HOME and resolve the full path.
function _Executor-ExpandPath {
    param([string]$Path)
    $expanded = $Path -replace '^~', $HOME
    return [System.IO.Path]::GetFullPath($expanded)
}

# _Executor-TrySymlink <Src> <Dst>
# Attempt to create a symbolic link. Returns $true on success.
function _Executor-TrySymlink {
    param([string]$Src, [string]$Dst)
    try {
        New-Item -ItemType SymbolicLink -Path $Dst -Target $Src -Force -ErrorAction Stop | Out-Null
        return $true
    } catch { return $false }
}

# _Executor-TryJunction <Src> <Dst>
# Attempt to create a directory junction (Windows-specific). Returns $true on success.
function _Executor-TryJunction {
    param([string]$Src, [string]$Dst)
    try {
        New-Item -ItemType Junction -Path $Dst -Target $Src -Force -ErrorAction Stop | Out-Null
        return $true
    } catch { return $false }
}

# _Executor-DeployFiles <Feature> <MetaFile> [<PlatformMetaFile>]
# Deploy files declared in feature.yaml and add fs resources to the active state patch.
# Supports op: link (symlink / junction with copy fallback) and op: copy.
function _Executor-DeployFiles {
    param(
        [Parameter(Mandatory=$true)]  [string]$Feature,
        [Parameter(Mandatory=$true)]  [string]$MetaFile,
        [string]$PlatformMetaFile = ""
    )

    $featureDir = _Executor-GetFeatureDir -Feature $Feature
    $allFiles  = @()
    $allFiles += _Executor-GetFilesJson -MetaFile $MetaFile
    $allFiles += _Executor-GetFilesJson -MetaFile $PlatformMetaFile

    if ($allFiles.Count -eq 0) { return $true }

    foreach ($entry in $allFiles) {
        $srcRel = $entry.src
        $op     = if ($entry.PSObject.Properties['op'] -and
                      -not [string]::IsNullOrWhiteSpace($entry.op)) { $entry.op } else { "link" }
        $target = _Executor-ExpandPath -Path $entry.target

        $src = Join-Path $featureDir "files\$srcRel"
        if (-not (Test-Path $src)) {
            Log-Error "executor: file source not found: $src"
            return $false
        }

        # Ensure parent directory exists
        $parent = [System.IO.Path]::GetDirectoryName($target)
        if ($parent -and -not (Test-Path $parent)) {
            New-Item -ItemType Directory -Path $parent -Force | Out-Null
        }

        # Handle conflict: remove if managed by this tool, fail otherwise
        if (Test-Path $target -ErrorAction SilentlyContinue) {
            if (State-HasFile -File $target) {
                Remove-Item -Path $target -Recurse -Force
            } else {
                Log-Error "executor: path exists and is not managed: $target"
                return $false
            }
        }

        $srcItem   = Get-Item $src
        $isDir     = $srcItem.PSIsContainer
        $deployed  = $false
        $entryType = "file"
        $actualOp  = "copy"

        if ($op -eq "link") {
            if (_Executor-TrySymlink -Src $src -Dst $target) {
                Log-Success "  Linked $target"
                $deployed  = $true
                $entryType = if ($isDir) { "dir" } else { "symlink" }
                $actualOp  = "link"
            } elseif ($isDir -and (_Executor-TryJunction -Src $src -Dst $target)) {
                Log-Success "  Junctioned $target"
                $deployed  = $true
                $entryType = "dir"
                $actualOp  = "link"
            }
        }

        if (-not $deployed) {
            # Fallback to copy
            Copy-Item -Path $src -Destination $target -Recurse -Force -ErrorAction Stop
            $suffix    = if ($op -eq "link") { " (link fallback)" } else { "" }
            Log-Success "  Copied $target$suffix"
            $entryType = if ($isDir) { "dir" } else { "file" }
            $actualOp  = "copy"
        }

        $resource = [PSCustomObject]@{
            kind = "fs"
            id   = "fs:$target"
            fs   = [PSCustomObject]@{ path = $target; entry_type = $entryType; op = $actualOp }
        }
        State-PatchAddResource -Feature $Feature -ResourceObject $resource
    }
    return $true
}

# _Executor-RemoveResources <Feature>
# Reverse of apply: reads current state and removes fs/runtime/package resources.
#
# Removal order (reverse of install): files → runtimes → packages
# Skip rules:
#   - Resources with backend="unknown" are NOT backend-uninstalled (legacy / pre-Phase4)
#   - Packages with managed:false in feature.yaml are NOT uninstalled
function _Executor-RemoveResources {
    param([Parameter(Mandatory=$true)] [string]$Feature)

    if (-not (State-HasFeature -Feature $Feature)) { return $true }

    $resources = @(State-QueryResources -Feature $Feature)
    if ($resources.Count -eq 0) { return $true }

    # 1. Remove fs resources (files / dirs / symlinks / junctions)
    foreach ($res in ($resources | Where-Object { $_.kind -eq "fs" })) {
        $path = $res.fs.path
        if (Test-Path $path -ErrorAction SilentlyContinue) {
            Log-Info "    removing: $path"
            Remove-Item -Path $path -Recurse -Force
        }
    }

    # 2. Uninstall managed runtimes (backend != "unknown")
    foreach ($res in ($resources | Where-Object { $_.kind -eq "runtime" })) {
        $backend = if ($res.PSObject.Properties['backend'] -and $res.backend) {
            $res.backend } else { "unknown" }
        if ($backend -eq "unknown") { continue }

        $rtName = $res.runtime.name
        $rtVer  = if ($res.runtime.PSObject.Properties['version'] -and $res.runtime.version) {
            [string]$res.runtime.version } else { "" }
        Log-Info "    uninstalling runtime: ${rtName}@${rtVer}"
        try {
            Load-Backend -BackendId $backend
            Backend-Call "UninstallRuntime" $rtName $rtVer
        } catch {
            Log-Warn "    uninstall_runtime failed for ${rtName}@${rtVer} (continuing): $_"
        }
    }

    # 3. Uninstall managed packages (backend != "unknown", managed != false)
    # use $Feature (canonical) for unmanaged check — passes to ResolveFeature which strips prefix
    foreach ($res in ($resources | Where-Object { $_.kind -eq "package" })) {
        $backend = if ($res.PSObject.Properties['backend'] -and $res.backend) {
            $res.backend } else { "unknown" }
        if ($backend -eq "unknown") { continue }

        $pkgName = $res.package.name
        if (-not (_Executor-PkgManaged -Feature $Feature -PkgName $pkgName)) {
            Log-Info "    skipping unmanaged package: $pkgName"
            continue
        }

        Log-Info "    uninstalling package: $pkgName"
        try {
            Load-Backend -BackendId $backend
            Backend-Call "UninstallPackage" $pkgName
        } catch {
            Log-Warn "    uninstall_package failed for ${pkgName} (continuing): $_"
        }
    }

    return $true
}

# ── Feature operations ────────────────────────────────────────────────────────

# _Executor-RunScript <ScriptPath> [<EnvVars>]
# Run a feature script as a subprocess. Returns $true on success, $false on failure.
function _Executor-RunScript {
    param(
        [Parameter(Mandatory=$true)] [string]$ScriptPath,
        [hashtable]$EnvVars = @{}
    )

    if (-not (Test-Path $ScriptPath)) {
        Log-Error "executor: script not found: $ScriptPath"
        return $false
    }

    try {
        # Set extra env vars for the subprocess and restore on exit
        $saved = @{}
        foreach ($kv in $EnvVars.GetEnumerator()) {
            $saved[$kv.Key] = [System.Environment]::GetEnvironmentVariable($kv.Key)
            [System.Environment]::SetEnvironmentVariable($kv.Key, $kv.Value)
        }

        & $ScriptPath
        $exitCode = $LASTEXITCODE

        foreach ($kv in $saved.GetEnumerator()) {
            [System.Environment]::SetEnvironmentVariable($kv.Key, $kv.Value)
        }

        return ($exitCode -eq 0 -or $null -eq $exitCode)
    } catch {
        Log-Error "executor: script raised exception: $ScriptPath — $_"
        return $false
    }
}

# _Executor-Install <Feature> [<ConfigVersion>]
# Full install pipeline:
#   1. Resolve meta files
#   2. State-PatchBegin
#   3. Install packages/runtimes/files from feature.yaml → state patch
#   4. State-PatchFinalize
#   5. Run install.ps1 (for secondary setup)
function _Executor-Install {
    param(
        [Parameter(Mandatory=$true)] [string]$Feature,
        [string]$ConfigVersion = ""
    )

    Log-Info "Installing: $Feature"

    $featureDir = _Executor-GetFeatureDir -Feature $Feature
    $metaFile     = _Executor-ResolveFeature -Feature $Feature
    $platformMeta = _Executor-ResolvePlatformFeature -Feature $Feature
    if ($null -eq $platformMeta) { $platformMeta = "" }

    State-PatchBegin

    if (-not (_Executor-ApplyPackages -Feature $Feature -MetaFile $metaFile -PlatformMetaFile $platformMeta)) {
        Log-Error "executor: package installation failed for: $Feature"
        return $false
    }

    if (-not (_Executor-ApplyRuntimes -Feature $Feature -MetaFile $metaFile -PlatformMetaFile $platformMeta -ConfigVersion $ConfigVersion)) {
        Log-Error "executor: runtime installation failed for: $Feature"
        return $false
    }

    if (-not (_Executor-DeployFiles -Feature $Feature -MetaFile $metaFile -PlatformMetaFile $platformMeta)) {
        Log-Error "executor: file deployment failed for: $Feature"
        return $false
    }

    State-PatchFinalize | Out-Null

    # Run install.ps1 for remaining setup (secondary packages, bootstrap logic, etc.)
    $script = Join-Path $featureDir "install.ps1"
    if (Test-Path $script) {
        $envVars = @{}
        if (-not [string]::IsNullOrWhiteSpace($ConfigVersion)) {
            $envVars["LOADOUT_FEATURE_CONFIG_VERSION"] = $ConfigVersion
        }
        if (-not (_Executor-RunScript -ScriptPath $script -EnvVars $envVars)) {
            Log-Error "executor: install script failed for: $Feature"
            return $false
        }
    }

    return $true
}

# _Executor-Destroy <Feature>
# Full destroy pipeline:
#   1. Remove resources tracked in state (fs + managed runtimes/packages)
#   2. Run uninstall.ps1 (for secondary cleanup)
#   3. State-PatchBegin → State-PatchRemoveFeature → State-PatchFinalize
function _Executor-Destroy {
    param([Parameter(Mandatory=$true)] [string]$Feature)

    Log-Info "Destroying: $Feature"

    $featureDir = _Executor-GetFeatureDir -Feature $Feature

    if (-not (_Executor-RemoveResources -Feature $Feature)) { return $false }

    $script = Join-Path $featureDir "uninstall.ps1"
    if (Test-Path $script) {
        if (-not (_Executor-RunScript -ScriptPath $script)) {
            Log-Error "executor: uninstall script failed for: $Feature"
            return $false
        }
    }

    State-PatchBegin
    State-PatchRemoveFeature -Feature $Feature
    State-PatchFinalize | Out-Null

    return $true
}

# _Executor-Replace <Feature> [<ConfigVersion>]
# Replace = destroy resources + uninstall script + state remove, then full install.
function _Executor-Replace {
    param(
        [Parameter(Mandatory=$true)] [string]$Feature,
        [string]$ConfigVersion = ""
    )

    Log-Info "Replacing: $Feature"

    $featureDir = _Executor-GetFeatureDir -Feature $Feature

    if (-not (_Executor-RemoveResources -Feature $Feature)) { return $false }

    $uninstallScript = Join-Path $featureDir "uninstall.ps1"
    if (Test-Path $uninstallScript) {
        if (-not (_Executor-RunScript -ScriptPath $uninstallScript)) {
            Log-Error "executor: uninstall script failed during replace for: $Feature"
            return $false
        }
    }

    State-PatchBegin
    State-PatchRemoveFeature -Feature $Feature
    State-PatchFinalize | Out-Null

    return (_Executor-Install -Feature $Feature -ConfigVersion $ConfigVersion)
}

# ── Plan reporting ────────────────────────────────────────────────────────────

function _Executor-ReportBlocked {
    param([object]$Plan)

    $blocked = @($Plan.blocked)
    if ($blocked.Count -gt 0) {
        Log-Warn "Skipping $($blocked.Count) blocked feature(s):"
        foreach ($b in $blocked) {
            Log-Warn "  ⊘ $($b.feature): $($b.reason)"
        }
    }
}

function _Executor-ReportSummary {
    param([object]$Plan)

    $s = $Plan.summary
    Log-Info "Plan: create=$($s.create)  destroy=$($s.destroy)  replace=$($s.replace)  noop=$($s.noop)  blocked=$($s.blocked)"
}

# ── Public API ────────────────────────────────────────────────────────────────

function Invoke-ExecutorRun {
    <#
    .SYNOPSIS Execute all actions in the given plan JSON string.
    Blocked features are skipped.
    Any action failure causes immediate abort (exit 1).
    #>
    param(
        [Parameter(Mandatory=$true)] [string]$PlanJson
    )

    if ([string]::IsNullOrWhiteSpace($PlanJson)) {
        Log-Error "Invoke-ExecutorRun: PlanJson is required"
        exit 1
    }

    $plan = $PlanJson | ConvertFrom-Json

    _Executor-ReportBlocked -Plan $plan
    _Executor-ReportSummary -Plan $plan

    $actions = @($plan.actions)

    if ($actions.Count -eq 0) {
        Log-Info "Nothing to do."
        return
    }

    Log-Task "Executing plan ($($actions.Count) actions)..."

    foreach ($action in $actions) {
        $feature       = $action.feature
        $operation     = $action.operation
        $configVersion = ""
        if ($action.details -and $action.details.PSObject.Properties['config_version'] -and
            $null -ne $action.details.config_version) {
            $configVersion = [string]$action.details.config_version
        }
        $actionDetails = if ($action.PSObject.Properties['details'] -and
                             $null -ne $action.details) { $action.details } else { $null }
        $featureMode   = _Executor-GetFeatureMode -Feature $feature

        $ok = switch ($operation) {
            "destroy" {
                if ($featureMode -eq "declarative") {
                    Invoke-DeclarativeExecutorRun -Feature $feature -Operation "destroy" -Details $actionDetails
                } else {
                    _Executor-Destroy -Feature $feature
                }
            }
            "create" {
                if ($featureMode -eq "declarative") {
                    Invoke-DeclarativeExecutorRun -Feature $feature -Operation "create" -Details $actionDetails
                } else {
                    _Executor-Install -Feature $feature -ConfigVersion $configVersion
                }
            }
            "replace" {
                if ($featureMode -eq "declarative") {
                    Invoke-DeclarativeExecutorRun -Feature $feature -Operation "replace" -Details $actionDetails
                } else {
                    _Executor-Replace -Feature $feature -ConfigVersion $configVersion
                }
            }
            "replace_backend" {
                if ($featureMode -eq "declarative") {
                    Invoke-DeclarativeExecutorRun -Feature $feature -Operation "replace_backend" -Details $actionDetails
                } else {
                    _Executor-Replace -Feature $feature -ConfigVersion $configVersion
                }
            }
            "strengthen" {
                if ($featureMode -eq "declarative") {
                    Invoke-DeclarativeExecutorRun -Feature $feature -Operation "strengthen" -Details $actionDetails
                } else {
                    Log-Error "executor: 'strengthen' is not supported for script-mode features: $feature"
                    $false
                }
            }
            default {
                Log-Error "executor: unknown operation '$operation' for feature '$feature'"
                $false
            }
        }

        if (-not $ok) {
            Log-Error "executor: aborting due to failure on feature: $feature"
            exit 1
        }
    }
}
