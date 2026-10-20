# Instala ou atualiza a CLI `terraforme` a partir deste repositório (Windows).
#
#   .\install.windows.ps1                 # compila o código local e instala em %LOCALAPPDATA%\Programs\terraforme\bin
#   .\install.windows.ps1 -Update         # git pull (fast-forward) antes de compilar
#   .\install.windows.ps1 -Dist           # build otimizada (LTO, mais lenta para compilar)
#   $env:PREFIX='C:\tools'; .\install.windows.ps1   # instala em C:\tools\bin
#   .\install.windows.ps1 -Uninstall      # remove (e tira a pasta do PATH do usuário)
#
# Se a execução de scripts estiver bloqueada:
#   powershell -ExecutionPolicy Bypass -File .\install.windows.ps1
#
# `terraforme upgrade` roda este script com -Update.
[CmdletBinding()]
param(
    [switch]$Update,
    [switch]$Dist,
    [switch]$Uninstall,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}

$BinName = 'terraforme.exe'
$Prefix = if ($env:PREFIX) { $env:PREFIX } else { Join-Path $env:LOCALAPPDATA 'Programs\terraforme' }
$BinDir = Join-Path $Prefix 'bin'
$Target = Join-Path $BinDir $BinName
# Daemon MCP compartilhado (conversa entre instâncias, subagentes), iniciado
# pelo `terraforme cli` a partir do PATH.
$McpTarget = Join-Path $BinDir 'terraforme-mcp.exe'
$RepoDir = $PSScriptRoot
$Profile_ = if ($Dist) { 'dist' } else { 'release' }

function Info($msg) { Write-Host '==> ' -ForegroundColor Blue -NoNewline; Write-Host $msg }
function Ok($msg)   { Write-Host '✔ ' -ForegroundColor Green -NoNewline; Write-Host $msg }
function Warn($msg) { Write-Host '! ' -ForegroundColor Yellow -NoNewline; Write-Host $msg }
function Die($msg)  { Write-Host '✘ ' -ForegroundColor Red -NoNewline; Write-Host $msg; exit 1 }

function Usage {
    Get-Content $PSCommandPath | Select-Object -Skip 1 -First 11 | ForEach-Object { $_ -replace '^# ?', '' }
}

if ($Help) { Usage; exit 0 }

# PATH do usuário (registro), não o do processo, que inclui o PATH do sistema.
function Get-UserPath { [Environment]::GetEnvironmentVariable('Path', 'User') }
function Test-InPath($path, $dir) {
    if (-not $path) { return $false }
    $dir = $dir.TrimEnd('\')
    foreach ($entry in $path.Split(';')) {
        if ($entry.TrimEnd('\') -ieq $dir) { return $true }
    }
    return $false
}

if ($Uninstall) {
    if (-not (Test-Path $Target)) { Die "$Target não existe — nada a remover." }
    Remove-Item -Force $Target
    Remove-Item -Force -ErrorAction SilentlyContinue "$Target.old", "$Target.new"
    Remove-Item -Force -ErrorAction SilentlyContinue $McpTarget, "$McpTarget.old", "$McpTarget.new"
    $userPath = Get-UserPath
    if (Test-InPath $userPath $BinDir) {
        $kept = $userPath.Split(';') | Where-Object { $_ -and ($_.TrimEnd('\') -ine $BinDir.TrimEnd('\')) }
        [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
        Ok "$BinDir removido do PATH do usuário"
    }
    Ok "Removido $Target"
    exit 0
}

if ([Environment]::OSVersion.Platform -ne 'Win32NT') { Die 'Este script é só para Windows (no Linux use install.linux.sh).' }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Die 'cargo não encontrado. Instale o Rust: https://rustup.rs (com o "Desktop development with C++" do Visual Studio Build Tools)'
}

if ($Update) {
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Die 'git não encontrado.' }
    git -C $RepoDir rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Warn 'A branch atual não tem upstream — compilando o código local sem git pull.'
    } else {
        $branch = git -C $RepoDir rev-parse --abbrev-ref HEAD
        Info "Atualizando o repositório ($branch)…"
        git -C $RepoDir pull --ff-only
        if ($LASTEXITCODE -ne 0) {
            Die "git pull falhou (alterações locais ou branch divergente). Resolva em $RepoDir e rode de novo."
        }
    }
}

$previous = ''
if (Test-Path $Target) {
    try { $previous = (& $Target version 2>$null | Out-String).Trim() } catch {}
}

Info "Compilando terraforme (perfil $Profile_)…"
cargo build --profile $Profile_ --locked -p terraforme-cli -p terraforme-mcp --manifest-path (Join-Path $RepoDir 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { Die 'cargo build falhou.' }

# O Windows não deixa sobrescrever um .exe em execução (o `terraforme upgrade`
# fica rodando enquanto este script roda), mas deixa renomeá-lo: o atual vira
# .old e o novo toma o lugar. O .old é apagado na próxima instalação.
function Install-Exe($name, $dest) {
    Remove-Item -Force -ErrorAction SilentlyContinue "$dest.old"
    Copy-Item -Force (Join-Path $RepoDir "target\$Profile_\$name") "$dest.new"
    if (Test-Path $dest) { Move-Item -Force $dest "$dest.old" }
    Move-Item -Force "$dest.new" $dest
    Remove-Item -Force -ErrorAction SilentlyContinue "$dest.old"
}

Info "Instalando em $Target"
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
Install-Exe $BinName $Target
Install-Exe 'terraforme-mcp.exe' $McpTarget
if (Get-Process -Name 'terraforme-mcp' -ErrorAction SilentlyContinue) {
    Warn 'terraforme-mcp antigo ainda rodando: feche os `terraforme cli` abertos (ou Stop-Process -Name terraforme-mcp) para o novo subir.'
}

$current = (& $Target version | Out-String).Trim()
if ($previous -eq $current) {
    Ok "Recompilado: $current (mesma versão)"
} elseif ($previous) {
    Ok "Atualizado: $previous → $current"
    # Seções do CHANGELOG.md mais novas que a versão anterior.
    $changelog = Join-Path $RepoDir 'CHANGELOG.md'
    if (Test-Path $changelog) {
        $prev = ($previous -split ' ')[-1]
        Write-Host ''
        $show = $false
        foreach ($line in Get-Content -Encoding UTF8 $changelog) {
            if ($line -match '^## (\S+)') {
                if ($Matches[1] -eq $prev) { break }
                $show = $true
            }
            if ($show) { Write-Host $line }
        }
    }
} else {
    Ok "Instalado: $current"
}

if (-not (Test-InPath (Get-UserPath) $BinDir)) {
    $userPath = Get-UserPath
    $newPath = if ($userPath) { "$($userPath.TrimEnd(';'));$BinDir" } else { $BinDir }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host ''
    Warn "$BinDir adicionado ao PATH do usuário. Abra um novo terminal para usar ``terraforme``."
}
