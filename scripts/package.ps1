$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
$package = Join-Path $repository 'target\release\Joy-Con-Bridge.exe'
$userProfile = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
$separator = [char]0x1f
$env:CARGO_ENCODED_RUSTFLAGS = @(
    "--remap-path-prefix=$repository=."
    "--remap-path-prefix=$userProfile=<home>"
) -join $separator

& cargo build --manifest-path (Join-Path $repository 'Cargo.toml') --release --bin Joy-Con-Bridge
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$bytes = [System.IO.File]::ReadAllBytes($package)
$ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
$unicode = [System.Text.Encoding]::Unicode.GetString($bytes)
foreach ($privatePath in @($userProfile, $repository)) {
    if ($ascii.Contains($privatePath) -or $unicode.Contains($privatePath)) {
        throw 'Joy-Con Bridge contains a private build path'
    }
}

$checksum = (Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath "$package.sha256" -Encoding ASCII -Value "$checksum  $(Split-Path -Leaf $package)"
Write-Output "Created $package"
