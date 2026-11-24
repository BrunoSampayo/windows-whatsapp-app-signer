param(
    [string]$Cargo = "cargo",
    [string]$Wix = "wix",
    [string]$WixUI = "WixToolset.UI.wixext"
)

$ErrorActionPreference = "Stop"
$Project = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $Project
try {
    & $Cargo build --release --locked --target x86_64-pc-windows-msvc --target-dir target
    if ($LASTEXITCODE -ne 0) { throw "A compilação do Assinador falhou." }

    $Executable = Join-Path $Project "target\x86_64-pc-windows-msvc\release\whatsapp.exe"
    $Output = Join-Path $Project "target\installer\Assinador-WhatsApp-x64.msi"
    & $Wix build (Join-Path $PSScriptRoot "Assinador.wxs") -arch x64 -ext $WixUI -culture pt-BR -d "AppExe=$Executable" -o $Output
    if ($LASTEXITCODE -ne 0) { throw "A geração do instalador MSI falhou." }
    Write-Output $Output
}
finally {
    Pop-Location
}
