# Deixa o terra disponível fora do Git Bash. Chamado pelo install-win.sh:
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File install-powershell.ps1 -BinDir <pasta>
# Põe a pasta do terra.exe no PATH do usuário (PowerShell, CMD e Windows
# Terminal abertos depois) e grava a função `terra` nos perfis do PowerShell,
# para o `terra workspace` entrar no diretório escolhido. Cada linha da saída
# vira uma nota no resumo do instalador.
param(
    [Parameter(Mandatory)] [string] $BinDir
)
$ErrorActionPreference = 'Stop'
# As notas vão para o bash por um pipe: em UTF-8, não na página OEM.
[Console]::OutputEncoding = [Text.Encoding]::UTF8

$beginMark = '# >>> terraforme >>>'
$endMark = '# <<< terraforme <<<'

# O PATH do usuário fica no registro como REG_EXPAND_SZ: lido sem expandir as
# variáveis (%USERPROFILE%...) e gravado com o mesmo tipo, para não trocá-las
# pelos valores de hoje.
function Add-UserPath {
    $key = Get-Item 'HKCU:\Environment'
    $current = $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    $entries = @($current -split ';' | Where-Object { $_ })
    $present = $entries | Where-Object {
        [Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -ieq $BinDir.TrimEnd('\')
    }
    if ($present) {
        "$BinDir já está no PATH do usuário"
        return
    }
    $value = (@($BinDir) + $entries) -join ';'
    Set-ItemProperty 'HKCU:\Environment' -Name Path -Value $value -Type ExpandString
    # Avisa as janelas (Explorer, Windows Terminal) que o ambiente mudou: o
    # SetEnvironmentVariable do .NET manda o WM_SETTINGCHANGE.
    [Environment]::SetEnvironmentVariable('TERRAFORME_PATH_REFRESH', '1', 'User')
    [Environment]::SetEnvironmentVariable('TERRAFORME_PATH_REFRESH', $null, 'User')
    "$BinDir adicionado ao PATH do usuário (PowerShell e CMD abertos depois)"
}

# A política que vale num PowerShell novo: a primeira definida, na ordem de
# precedência, sem a do processo (que aqui é o Bypass). Sem nenhuma, um
# Windows cliente usa a Restricted.
function Get-ProfilePolicy {
    foreach ($scope in 'MachinePolicy', 'UserPolicy', 'CurrentUser', 'LocalMachine') {
        $policy = Get-ExecutionPolicy -Scope $scope
        if ($policy -ne 'Undefined') { return $policy }
    }
    'Restricted'
}

# A função usa o caminho completo do terra.exe, não o PATH: assim funciona
# já na sessão em que o perfil carrega.
# O caminho vai entre aspas simples: um ' (de O'Brien) vira ''.
$terraExe = (Join-Path $BinDir 'terra.exe').Replace("'", "''")
$block = @"
function terra {
    `$tmp = New-TemporaryFile
    try {
        & '$terraExe' @args --cwd-file `$tmp.FullName
        `$rc = `$LASTEXITCODE
        `$dir = Get-Content -LiteralPath `$tmp.FullName -Raw -ErrorAction SilentlyContinue
        if (`$dir -and (Test-Path -LiteralPath `$dir -PathType Container)) {
            Set-Location -LiteralPath `$dir
        }
    } finally {
        Remove-Item -LiteralPath `$tmp.FullName -ErrorAction SilentlyContinue
    }
    `$global:LASTEXITCODE = `$rc
}
"@

# Remove um bloco antigo (entre os marcadores) e grava o novo no fim do perfil.
function Write-ProfileBlock([string] $file) {
    $lines = @()
    if (Test-Path -LiteralPath $file) {
        $skip = $false
        foreach ($line in Get-Content -LiteralPath $file -Encoding UTF8) {
            if ($line -eq $beginMark) { $skip = $true }
            if (-not $skip) { $lines += $line }
            if ($line -eq $endMark) { $skip = $false }
        }
    } else {
        New-Item -ItemType Directory -Force (Split-Path $file) | Out-Null
    }
    $lines += $beginMark, $block, $endMark
    # No PowerShell 5.1, o UTF8 grava com BOM, e o perfil é lido como UTF-8.
    $lines | Set-Content -LiteralPath $file -Encoding UTF8
    "Função 'terra' adicionada em $file"
}

Add-UserPath

$policy = Get-ProfilePolicy
if ($policy -in 'Restricted', 'AllSigned') {
    # Com essas políticas o PowerShell não carrega o perfil (e mostraria um
    # erro a cada janela): o terra fica só no PATH.
    "aviso: a política de execução do PowerShell ($policy) não carrega o perfil; o terra funciona, mas o 'terra workspace' só mostra o path escolhido. Para entrar nele, rode 'Set-ExecutionPolicy -Scope CurrentUser RemoteSigned' e o instalador de novo."
    return
}

# O perfil do Windows PowerShell 5.1 e o do PowerShell 7, na pasta
# Documentos (que pode estar no OneDrive).
$documents = [Environment]::GetFolderPath('MyDocuments')
foreach ($dir in 'WindowsPowerShell', 'PowerShell') {
    Write-ProfileBlock (Join-Path $documents "$dir\Microsoft.PowerShell_profile.ps1")
}
