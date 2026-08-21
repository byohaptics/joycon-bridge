$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($env:PUBLICATION_DENYLIST)) {
    throw 'PUBLICATION_DENYLIST is required for publication checks.'
}

& git grep --cached -n -I -i -E -- $env:PUBLICATION_DENYLIST
if ($LASTEXITCODE -eq 0) { throw 'Publication policy check failed.' }
if ($LASTEXITCODE -ne 1) { exit $LASTEXITCODE }
Write-Output 'Publication policy check passed.'
