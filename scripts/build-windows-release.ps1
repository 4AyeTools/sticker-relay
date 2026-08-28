param(
  [Parameter(Mandatory = $true)][string]$Target,
  [Parameter(Mandatory = $true)][string]$Label
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $projectRoot

$version = node -p "require('./package.json').version"
$releaseRoot = Join-Path $projectRoot "src-tauri\target\$Target\release"
$binaryPath = Join-Path $releaseRoot 'sticker-relay.exe'
$releaseOutput = Join-Path $projectRoot 'release'
New-Item -ItemType Directory -Force -Path $releaseOutput | Out-Null

npm run build
& cargo build --manifest-path src-tauri/Cargo.toml --release --target $Target --features custom-protocol
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$importedCertificate = $null
if ($env:WINDOWS_CERTIFICATE_BASE64 -and $env:WINDOWS_CERTIFICATE_PASSWORD) {
  $certificatePath = Join-Path $env:RUNNER_TEMP 'sticker-relay-signing.pfx'
  [IO.File]::WriteAllBytes($certificatePath, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE_BASE64))
  $password = ConvertTo-SecureString $env:WINDOWS_CERTIFICATE_PASSWORD -AsPlainText -Force
  $importedCertificate = Import-PfxCertificate -FilePath $certificatePath -CertStoreLocation Cert:\CurrentUser\My -Password $password
  if (-not $importedCertificate) { throw 'Windows signing certificate import failed' }
  $signTool = Get-ChildItem -LiteralPath "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Filter signtool.exe -Recurse |
    Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
  if (-not $signTool) { throw 'signtool.exe was not found' }
  & $signTool.FullName sign /sha1 $importedCertificate.Thumbprint /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $binaryPath
  if ($LASTEXITCODE -ne 0) { throw 'application Authenticode signing failed' }
}

$bundleArguments = @('tauri', 'bundle', '--target', $Target, '--bundles', 'nsis', '--features', 'custom-protocol', '--ci')
if ($env:TAURI_SIGNING_PRIVATE_KEY) {
  $bundleArguments += @('--config', 'src-tauri/tauri.updater.conf.json')
}
npx @bundleArguments
if ($LASTEXITCODE -ne 0) { throw 'tauri bundle failed' }

$installer = Get-ChildItem -LiteralPath (Join-Path $releaseRoot 'bundle\nsis') -Filter "*_${version}_*-setup.exe" |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1
if (-not $installer) { throw 'NSIS installer was not generated' }

if ($importedCertificate) {
  & $signTool.FullName sign /sha1 $importedCertificate.Thumbprint /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $installer.FullName
  if ($LASTEXITCODE -ne 0) { throw 'installer Authenticode signing failed' }
}

$installerName = "sticker-relay-$version-$Label-setup.exe"
Copy-Item -LiteralPath $installer.FullName -Destination (Join-Path $releaseOutput $installerName) -Force

if ($env:TAURI_SIGNING_PRIVATE_KEY) {
  $updaterSignature = "$($installer.FullName).sig"
  if (-not (Test-Path -LiteralPath $updaterSignature)) {
    throw 'Tauri updater signature was not generated for the Windows installer'
  }
  Copy-Item -LiteralPath $updaterSignature -Destination (Join-Path $releaseOutput "$installerName.sig") -Force
}

$portableRoot = Join-Path ([IO.Path]::GetTempPath()) "sticker-relay-$version-$Label-portable-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path $portableRoot | Out-Null
Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $portableRoot 'sticker-relay.exe') -Force
Copy-Item -LiteralPath LICENSE, NOTICE, THIRD_PARTY_LICENSES.md -Destination $portableRoot -Force
Copy-Item -LiteralPath distribution\PORTABLE_README.txt -Destination (Join-Path $portableRoot 'README.txt') -Force
Compress-Archive -Path (Join-Path $portableRoot '*') -DestinationPath (Join-Path $releaseOutput "sticker-relay-$version-$Label-portable.zip") -Force

if ($importedCertificate) {
  Remove-Item -LiteralPath "Cert:\CurrentUser\My\$($importedCertificate.Thumbprint)" -Force
}
