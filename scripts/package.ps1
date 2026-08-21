$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content -LiteralPath (Join-Path $repository 'Cargo.toml') -Raw -Encoding UTF8
$version = [regex]::Match($manifest, '(?m)^version = "([^"]+)"$').Groups[1].Value
if (-not $version) { throw 'Cargo package version is missing' }

$build = Join-Path $repository 'build'
$package = Join-Path $build "BYO-Haptics-Joy-Con-Bridge-v$version.exe"
$userProfile = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
$separator = [char]0x1f
$env:CARGO_ENCODED_RUSTFLAGS = @(
    "--remap-path-prefix=$repository=."
    "--remap-path-prefix=$userProfile=<home>"
) -join $separator

& cargo build --manifest-path (Join-Path $repository 'Cargo.toml') --release --bin joycon-rumble-gui
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$source = Join-Path $repository 'target\release\joycon-rumble-gui.exe'
$bytes = [System.IO.File]::ReadAllBytes($source)
$ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
$unicode = [System.Text.Encoding]::Unicode.GetString($bytes)
foreach ($privatePath in @($userProfile, $repository)) {
    if ($ascii.Contains($privatePath) -or $unicode.Contains($privatePath)) {
        throw 'Joy-Con Bridge contains a private build path'
    }
}

if (-not (Test-Path -LiteralPath $build)) { New-Item -ItemType Directory -Path $build | Out-Null }
Copy-Item -LiteralPath $source -Destination $package -Force
$checksum = (Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath "$package.sha256" -Encoding ASCII -Value "$checksum  $(Split-Path -Leaf $package)"
Write-Output "Created $package"
