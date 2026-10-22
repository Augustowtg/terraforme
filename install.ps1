# Instala ou atualiza o terraforme a partir das releases do GitHub (sem Rust).
#
#   irm https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.ps1 | iex
#
# $env:TERRAFORME_VERSION = 'v0.19.0'   versao a instalar (padrao: a ultima release)
# $env:PREFIX = 'C:\tools'              instala em C:\tools\bin (padrao: %LOCALAPPDATA%\Programs\terraforme)
#
# So ASCII neste arquivo: o `irm | iex` e o PowerShell 5.1 sem BOM leriam
# acentos errado. Para compilar a partir do codigo, use o install.windows.ps1.

# Bloco proprio: o `iex` roda na sessao do usuario, entao nada de `exit`
# (fecharia o terminal) nem variaveis soltas.
& {
    $ErrorActionPreference = 'Stop'
    # Sem a barra de progresso o Invoke-WebRequest do PowerShell 5.1 e muito mais rapido.
    $ProgressPreference = 'SilentlyContinue'
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $Repo = 'Augustowtg/terraforme'
    $Target = 'x86_64-pc-windows-msvc'
    $Prefix = if ($env:PREFIX) { $env:PREFIX } else { Join-Path $env:LOCALAPPDATA 'Programs\terraforme' }
    $BinDir = Join-Path $Prefix 'bin'
    $Exe = Join-Path $BinDir 'terraforme.exe'
    $McpExe = Join-Path $BinDir 'terraforme-mcp.exe'

    function Info($msg) { Write-Host '==> ' -ForegroundColor Blue -NoNewline; Write-Host $msg }
    function Ok($msg) { Write-Host 'OK  ' -ForegroundColor Green -NoNewline; Write-Host $msg }
    function Warn($msg) { Write-Host '!   ' -ForegroundColor Yellow -NoNewline; Write-Host $msg }

    if ($env:OS -ne 'Windows_NT') { throw 'Este script e so para Windows (no Linux use o install.sh).' }

    $tag = $env:TERRAFORME_VERSION
    if (-not $tag) {
        $latest = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'terraforme-install' }
        $tag = $latest.tag_name
    }
    if (-not $tag.StartsWith('v')) { $tag = "v$tag" }
    $version = $tag.Substring(1)

    $previous = ''
    if (Test-Path $Exe) {
        try { $previous = (& $Exe version 2>$null | Out-String).Trim() } catch {}
    }

    if ($previous -eq "terraforme $version" -and (Test-Path $McpExe)) {
        Ok "Ja esta na versao $version ($Exe)"
    } else {
        $asset = "terraforme-$Target.zip"
        $url = "https://github.com/$Repo/releases/download/$tag/$asset"
        $tmp = Join-Path ([IO.Path]::GetTempPath()) ("terraforme-" + [guid]::NewGuid())
        New-Item -ItemType Directory -Force -Path $tmp | Out-Null
        try {
            Info "Baixando terraforme $version ($Target)..."
            $zip = Join-Path $tmp $asset
            Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $zip
            Invoke-WebRequest -UseBasicParsing -Uri "$url.sha256" -OutFile "$zip.sha256"
            $expected = ((Get-Content "$zip.sha256" -Raw).Trim() -split '\s+')[0].ToLower()
            $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
            if ($expected -ne $actual) { throw "O sha256 de $asset nao confere." }
            $out = Join-Path $tmp 'out'
            Expand-Archive -Force -Path $zip -DestinationPath $out

            # O Windows nao deixa sobrescrever um .exe em execucao, mas deixa
            # renomea-lo: o atual vira .old e o novo toma o lugar.
            Info "Instalando em $BinDir"
            New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
            foreach ($name in 'terraforme.exe', 'terraforme-mcp.exe') {
                $dest = Join-Path $BinDir $name
                Remove-Item -Force -ErrorAction SilentlyContinue "$dest.old"
                Copy-Item -Force (Join-Path $out $name) "$dest.new"
                if (Test-Path $dest) { Move-Item -Force $dest "$dest.old" }
                Move-Item -Force "$dest.new" $dest
                Remove-Item -Force -ErrorAction SilentlyContinue "$dest.old"
            }
        } finally {
            Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tmp
        }

        if ($previous) {
            Ok "Atualizado: $previous -> terraforme $version"
            Write-Host "    novidades: https://github.com/$Repo/releases/tag/$tag"
        } else {
            Ok "Instalado: terraforme $version"
        }
        # Daemon MCP da versao antiga no ar: reinicia com o binario novo (os
        # `terraforme cli` abertos se reconectam sozinhos). TERRAFORME_RESTART=1
        # reinicia sem perguntar (`terraforme upgrade --restart`), 0 nao reinicia.
        if (Get-Process -Name 'terraforme-mcp' -ErrorAction SilentlyContinue) {
            $restart = $env:TERRAFORME_RESTART
            if (-not $restart -and [Environment]::UserInteractive) {
                $answer = Read-Host 'Reiniciar o terraforme agora (daemon MCP)? [S/n]'
                $restart = if ($answer -match '^[NnTt]') { '0' } else { '1' }
            }
            if ($restart -eq '1') {
                & $Exe restart
                if ($LASTEXITCODE -ne 0) { Warn 'Falha ao reiniciar: rode `terraforme restart`.' }
            } else {
                Warn 'terraforme-mcp antigo ainda rodando: rode `terraforme restart` para subir o novo.'
            }
        }
    }

    # PATH do usuario (registro) e o desta sessao, para o `terraforme` ja funcionar aqui.
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = if ($userPath) { $userPath.Split(';') | ForEach-Object { $_.TrimEnd('\') } } else { @() }
    if ($entries -notcontains $BinDir.TrimEnd('\')) {
        $newPath = if ($userPath) { "$($userPath.TrimEnd(';'));$BinDir" } else { $BinDir }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Ok "$BinDir adicionado ao PATH do usuario"
    }
    if (($env:Path.Split(';') | ForEach-Object { $_.TrimEnd('\') }) -notcontains $BinDir.TrimEnd('\')) {
        $env:Path = "$BinDir;$env:Path"
    }

    if (-not $previous) {
        Write-Host ''
        Write-Host 'Proximo passo: terraforme setup'
    }
}
