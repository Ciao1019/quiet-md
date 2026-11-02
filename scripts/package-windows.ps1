param([string]$Iscc = 'ISCC.exe')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
    $version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $stage = Join-Path $root 'dist/windows/app'
    New-Item -ItemType Directory -Force $stage | Out-Null
    Copy-Item target/release/quiet_md.exe $stage -Force
    if (!$env:VCToolsRedistDir) { throw 'Run from a Visual Studio developer shell to locate the C++ runtime' }
    $runtime = Get-ChildItem (Join-Path $env:VCToolsRedistDir 'x64/Microsoft.VC*.CRT/vcruntime140.dll') | Select-Object -First 1
    if (!$runtime) { throw 'The redistributable x64 vcruntime140.dll was not found' }
    Copy-Item $runtime.FullName $stage -Force
    & $Iscc "/DAppVersion=$version" "/DAppSource=$stage" packaging/windows/quiet-md.iss
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
    $installer = Join-Path $root "dist/windows/quiet-md-$version-windows-x64-setup.exe"
    $hash = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $(Split-Path $installer -Leaf)" | Set-Content "$installer.sha256" -Encoding ascii
    Write-Output "Unsigned installer: $installer"
} finally { Pop-Location }
