#!/usr/bin/env pwsh
# Compila la APK companion de IA Optimizer.
# Uso:
#   ./build_app.ps1                       # usa JDK/Gradle del sistema si existen
#   ./build_app.ps1 -JavaHome <ruta> -GradleHome <ruta> -AndroidHome <ruta>
param(
    [string]$JavaHome = $env:JAVA_HOME,
    [string]$GradleHome = "",
    [string]$AndroidHome = $env:ANDROID_HOME
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path

if (-not $AndroidHome) {
    $AndroidHome = "$env:LOCALAPPDATA\Android\Sdk"
}
if (-not (Test-Path $AndroidHome)) {
    Write-Error "ANDROID_HOME no valido: $AndroidHome"
}

if (-not $JavaHome -or -not (Test-Path "$JavaHome\bin\java.exe")) {
    # Busca un JDK 17 portatil junto al script (toolchain/jdk/jdk-17*).
    $jdk = Get-ChildItem -Path (Join-Path $root "toolchain\jdk") -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "jdk-17*" } | Select-Object -First 1
    if ($jdk) { $JavaHome = $jdk.FullName }
}
if (-not $JavaHome -or -not (Test-Path "$JavaHome\bin\java.exe")) {
    Write-Error "JDK 17 no encontrado. Pasa -JavaHome o define JAVA_HOME."
}

if (-not $GradleHome) {
    $g = Get-ChildItem -Path (Join-Path $root "toolchain\gradle") -Directory -ErrorAction SilentlyContinue |
        Where-Object { Test-Path (Join-Path $_.FullName "bin\gradle.bat") } | Select-Object -First 1
    if ($g) { $GradleHome = $g.FullName }
}
if (-not $GradleHome) {
    $gradleCmd = Get-Command gradle -ErrorAction SilentlyContinue
    if (-not $gradleCmd) { Write-Error "Gradle no encontrado. Pasa -GradleHome." }
    $gradleCmd = $gradleCmd.Source
} else {
    $gradleCmd = Join-Path $GradleHome "bin\gradle.bat"
}

$env:JAVA_HOME = $JavaHome
$env:ANDROID_HOME = $AndroidHome
$env:ANDROID_SDK_ROOT = $AndroidHome
$env:PATH = "$JavaHome\bin;$env:PATH"

Write-Host "== IA Optimizer companion ==" -ForegroundColor Cyan
Write-Host "JDK:     $JavaHome"
Write-Host "Gradle:  $gradleCmd"
Write-Host "SDK:     $AndroidHome"

& $gradleCmd -p $root assembleDebug
if ($LASTEXITCODE -ne 0) { Write-Error "gradle fallo (exit $LASTEXITCODE)" }

$apk = Join-Path $root "app\build\outputs\apk\debug\app-debug.apk"
if (Test-Path $apk) {
    Write-Host "== OK == $apk" -ForegroundColor Green
} else {
    Write-Error "No se genero el APK"
}
