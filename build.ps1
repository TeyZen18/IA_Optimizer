# IA Optimizer - build.ps1
# Compila el demonio para Android arm64 y empaqueta el modulo .zip
# Uso:  powershell -ExecutionPolicy Bypass -File build.ps1

$ErrorActionPreference = "Stop"
$root   = Split-Path -Parent $MyInvocation.MyCommand.Path
$ndk    = "$env:LOCALAPPDATA\Android\Sdk\ndk\30.0.16248370"
$clang  = "$ndk\toolchains\llvm\prebuilt\windows-x86_64\bin\aarch64-linux-android21-clang.cmd"

if (-not (Test-Path $clang)) { Write-Error "No se encontro el NDK en $ndk"; exit 1 }

$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = $clang

Write-Host "== compilando iaopt (aarch64-linux-android) =="
Push-Location $root
cargo build --release --target aarch64-linux-android -p iaopt-daemon
if ($LASTEXITCODE -ne 0) { Pop-Location; Write-Error "cargo build fallo (exit $LASTEXITCODE)"; exit 1 }
Pop-Location

$stage = "$root\build\module"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path "$stage\bin" | Out-Null

foreach ($f in @("module.prop","action.sh","customize.sh","post-fs-data.sh","service.sh","uninstall.sh","NOTICE.md")) {
    Copy-Item "$root\$f" "$stage\$f" -Force
}

$bin = "$root\target\aarch64-linux-android\release\iaopt"
Copy-Item $bin "$stage\bin\iaopt" -Force

# APK companion dentro del modulo: se instala como app de sistema (system/app).
$apk = "$root\IAOptimizerApp\app\build\outputs\apk\debug\app-debug.apk"
if (-not (Test-Path $apk)) { Write-Error "No se encontro la APK companion: $apk"; exit 1 }
New-Item -ItemType Directory -Force -Path "$stage\system\app\IAOptimizer" | Out-Null
Copy-Item $apk "$stage\system\app\IAOptimizer\IAOptimizer.apk" -Force
Write-Host "apk: $apk ($((Get-Item $apk).Length) bytes)"

# LF para los scripts (Magisk los ejecuta con sh)
foreach ($f in @("action.sh","customize.sh","post-fs-data.sh","service.sh","uninstall.sh")) {
    $p = "$stage\$f"
    (Get-Content $p -Raw) -replace "`r`n", "`n" | Set-Content $p -NoNewline
}

$ver   = (Select-String -Path "$root\module.prop" -Pattern '^version=(.+)$').Matches.Groups[1].Value
$zip   = "$root\build\IA_Optimizer-$ver.zip"

# tar.exe (bsdtar) escribe entradas con separador '/' como espera Magisk;
# Compress-Archive de .NET usa '\' y romperia el flasheo.
if (Test-Path "$env:SystemRoot\System32\tar.exe") {
    & "$env:SystemRoot\System32\tar.exe" -a -c -f $zip -C $stage module.prop action.sh customize.sh post-fs-data.sh service.sh uninstall.sh NOTICE.md bin system
} else {
    Write-Error "tar.exe no encontrado; no se puede empaquetar con rutas correctas"
    exit 1
}

Write-Host ""
Write-Host "== OK ==" -ForegroundColor Green
Write-Host "zip: $zip"
Write-Host "bin: $bin ($((Get-Item $bin).Length) bytes)"
