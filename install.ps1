# Instala o terraforme no Windows direto do GitHub:
#   irm https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.ps1 | iex
# O terra roda no Git Bash: este script acha o bash do Git for Windows e roda
# nele o install.sh, que baixa o código e chama o install-win.sh.
$ErrorActionPreference = 'Stop'

$ref = if ($env:TERRAFORME_REF) { $env:TERRAFORME_REF } else { 'main' }
$url = "https://raw.githubusercontent.com/Augustowtg/terraforme/$ref/install.sh"

$bash = @(
    "$env:ProgramFiles\Git\bin\bash.exe",
    "${env:ProgramFiles(x86)}\Git\bin\bash.exe",
    "$env:LOCALAPPDATA\Programs\Git\bin\bash.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $bash) {
    $git = Get-Command git -ErrorAction SilentlyContinue
    if ($git) {
        $candidate = Join-Path (Split-Path (Split-Path $git.Source)) 'bin\bash.exe'
        if (Test-Path $candidate) { $bash = $candidate }
    }
}
if (-not $bash) {
    Write-Error "Git Bash não encontrado. Instale o Git for Windows (https://git-scm.com/download/win) e rode de novo."
    return
}

$script = Join-Path $env:TEMP "terraforme-install-$PID.sh"
Invoke-RestMethod $url -OutFile $script
try {
    & $bash $script @args
} finally {
    Remove-Item $script -ErrorAction SilentlyContinue
}
