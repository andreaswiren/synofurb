<#
.SYNOPSIS
    synofurb desktop installer for Windows (PowerShell 5.1 and 7+).

.DESCRIPTION
    Finds the release (latest, or -Version), downloads the MSI (default) or the portable zip, verifies the
    SHA-256 against the release's SHA256SUMS and - when minisign is installed - the minisign signature of that
    file, then installs. Nothing downloaded is ever evaluated; only verified files are executed.

    One-liner (latest release, MSI, machine-wide; asks for elevation only for msiexec):
        irm https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.ps1 | iex

    With parameters (a piped script cannot take any, so build a script block):
        & ([scriptblock]::Create((irm https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.ps1))) -Version 0.2.0 -Run scan
        & ([scriptblock]::Create((irm https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.ps1))) -User -Run install,--host,192.168.1.20

.PARAMETER Version
    Release to install, e.g. 0.2.0 (default: the latest release).
.PARAMETER Portable
    Download the portable zip and just extract it to -Dir (default %LOCALAPPDATA%\Programs\synofurb-portable). No
    shortcuts, no registry entries, no elevation.
.PARAMETER User
    Per-user install without elevation: extracts the zip to %LOCALAPPDATA%\Programs\synofurb, adds a Start Menu
    shortcut and an "Apps & features" uninstall entry for the current user. (The MSI is machine-wide only.)
.PARAMETER Dir
    Install directory. For the MSI it is passed as INSTALLDIR; for -User/-Portable it is the extraction folder.
.PARAMETER AddToPath
    With -User/-Portable: add the install folder to the current user's PATH.
.PARAMETER Run
    After installing, run `synofurb <Run...>`, e.g. -Run scan  or  -Run install,--host,192.168.1.20
.PARAMETER Uninstall
    Remove synofurb (MSI product, or the -User/-Portable folder, shortcut and uninstall entry).
.PARAMETER DryRun
    Show what would be done; download nothing, change nothing.
.PARAMETER InsecureSkipSignature
    Do not even try the minisign check (SHA-256 only).
.PARAMETER RequireSignature
    Fail unless the minisign signature could be verified.
.PARAMETER PublicKey
    minisign public key (base64 line) to verify SHA256SUMS.minisig with.
#>
[CmdletBinding()]
param(
    [string]$Version = '',
    [switch]$Portable,
    [switch]$User,
    [string]$Dir = '',
    [switch]$AddToPath,
    [string[]]$Run = @(),
    [switch]$Uninstall,
    [switch]$DryRun,
    [switch]$InsecureSkipSignature,
    [switch]$RequireSignature,
    [string]$PublicKey = ''
)

function Install-Synofurb {
    [CmdletBinding()]
    param(
        [string]$Version, [bool]$Portable, [bool]$User, [string]$Dir, [bool]$AddToPath, [string[]]$Run,
        [bool]$Uninstall, [bool]$DryRun, [bool]$InsecureSkipSignature, [bool]$RequireSignature, [string]$PublicKey
    )

    # function-scoped on purpose: when this script is piped into iex they must not leak into the user's session
    $ErrorActionPreference = 'Stop'
    Set-StrictMode -Version 2.0
    $ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is very slow in 5.1 with the progress bar

    $Repo = 'andreaswiren/synofurb'
    $ApiBase = if ($env:SYNOFURB_API_BASE) { $env:SYNOFURB_API_BASE.TrimEnd('/') } else { "https://api.github.com/repos/$Repo" }
    $DlBase = if ($env:SYNOFURB_DL_BASE) { $env:SYNOFURB_DL_BASE.TrimEnd('/') } else { "https://github.com/$Repo/releases/download" }
    # Replaced by the release procedure with the real minisign public key (see docs/RELEASING.md).
    $BuiltinKey = 'REPLACE_WITH_MINISIGN_PUBLIC_KEY'
    if (-not $PublicKey) { $PublicKey = if ($env:SYNOFURB_MINISIGN_PUBKEY) { $env:SYNOFURB_MINISIGN_PUBKEY } else { $BuiltinKey } }
    $MinisignName = if ($env:SYNOFURB_MINISIGN_BIN) { $env:SYNOFURB_MINISIGN_BIN } else { 'minisign' }
    $TestMode = ($env:SYNOFURB_PS_TEST -eq '1')   # lets the logic run on Linux/macOS CI against a fake server

    # ---- platform --------------------------------------------------------------------------------------
    $onWindows = ($PSVersionTable.PSEdition -eq 'Desktop') -or ((Get-Variable -Name IsWindows -ValueOnly -ErrorAction SilentlyContinue) -eq $true)
    if (-not $onWindows -and -not $TestMode) {
        throw 'install.ps1 is for Windows. On Linux and macOS use: curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh'
    }

    # TLS 1.2+ for Windows PowerShell 5.1 (it defaults to TLS 1.0/1.1 on older builds)
    try {
        $proto = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        try { $proto = $proto -bor [Net.SecurityProtocolType]'Tls13' } catch { Write-Verbose 'TLS 1.3 enum not available' }
        [Net.ServicePointManager]::SecurityProtocol = $proto
    } catch { Write-Warning "could not force TLS 1.2: $($_.Exception.Message)" }

    if ("$ApiBase$DlBase" -like 'http://*') { Write-Warning 'using a plain-http mirror (SYNOFURB_API_BASE / SYNOFURB_DL_BASE); this is meant for tests only' }
    elseif ($ApiBase -notlike 'https://*' -or $DlBase -notlike 'https://*') { throw 'SYNOFURB_API_BASE and SYNOFURB_DL_BASE must be https:// URLs' }

    # ---- helpers ---------------------------------------------------------------------------------------
    function Test-Admin {
        if ($TestMode -and -not $onWindows) { return $true }
        $id = [Security.Principal.WindowsIdentity]::GetCurrent()
        return (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    }
    function Get-Json([string]$Url) {
        try { return Invoke-RestMethod -UseBasicParsing -Uri $Url -Headers @{ 'User-Agent' = 'synofurb-install.ps1'; 'Accept' = 'application/vnd.github+json' } }
        catch { throw "could not query $Url ($($_.Exception.Message)). No release yet, offline, or GitHub API rate limit? Try -Version X.Y.Z" }
    }
    function Save-File([string]$Url, [string]$Path) {
        try { Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Path -Headers @{ 'User-Agent' = 'synofurb-install.ps1' } }
        catch { throw "download failed: $Url ($($_.Exception.Message))" }
    }
    function Get-InstalledEntries {
        # Add/Remove Programs entries named exactly "synofurb" (MSI product or our own per-user entry)
        $roots = @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall',
            'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall')
        $found = @()
        if (-not $onWindows) { return $found }
        foreach ($r in $roots) {
            if (-not (Test-Path -LiteralPath $r)) { continue }
            foreach ($k in Get-ChildItem -LiteralPath $r -ErrorAction SilentlyContinue) {
                $p = Get-ItemProperty -LiteralPath $k.PSPath -ErrorAction SilentlyContinue
                if ($p -and ($p.PSObject.Properties.Name -contains 'DisplayName') -and $p.DisplayName -eq 'synofurb') {
                    $isMsi = ($p.PSObject.Properties.Name -contains 'WindowsInstaller') -and ($p.WindowsInstaller -eq 1)
                    $loc = if ($p.PSObject.Properties.Name -contains 'InstallLocation') { $p.InstallLocation } else { '' }
                    $found += [pscustomobject]@{ Key = $k.PSPath; Name = $k.PSChildName; Msi = $isMsi; Location = $loc; Hive = $r.Substring(0, 4) }
                }
            }
        }
        return $found
    }
    function Find-Exe([string]$Hint) {
        $cands = @()
        if ($Hint) { $cands += (Join-Path $Hint 'synofurb.exe'); $cands += (Join-Path $Hint 'synofurb') }
        if ($env:ProgramFiles) { $cands += (Join-Path $env:ProgramFiles 'synofurb\synofurb.exe') }
        if (${env:ProgramFiles(x86)}) { $cands += (Join-Path ${env:ProgramFiles(x86)} 'synofurb\synofurb.exe') }
        if ($env:LOCALAPPDATA) { $cands += (Join-Path $env:LOCALAPPDATA 'Programs\synofurb\synofurb.exe') }
        foreach ($e in Get-InstalledEntries) { if ($e.Location) { $cands += (Join-Path $e.Location 'synofurb.exe') } }
        foreach ($c in $cands) { if ($c -and (Test-Path -LiteralPath $c -PathType Leaf)) { return $c } }
        $cmd = Get-Command synofurb -ErrorAction SilentlyContinue
        if ($cmd) { return $cmd.Source }
        return $null
    }
    function Invoke-Msiexec([string[]]$MsiArgs) {
        # elevate only msiexec, and only when we are not already an administrator
        $exe = if ($env:SYNOFURB_MSIEXEC) { $env:SYNOFURB_MSIEXEC } else { 'msiexec.exe' }
        if (Test-Admin) {
            & $exe @MsiArgs
            return $LASTEXITCODE
        }
        Write-Host 'Administrator rights are needed to install for all users; a Windows elevation prompt will appear ...'
        $argLine = ($MsiArgs | ForEach-Object { if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ } }) -join ' '
        $proc = Start-Process -FilePath $exe -ArgumentList $argLine -Verb RunAs -Wait -PassThru
        return $proc.ExitCode
    }
    function Assert-MsiOk([int]$Code, [string]$What, [string]$Log) {
        if ($Code -in 0, 3010, 1641) {
            if ($Code -ne 0) { Write-Warning "$What finished; a restart is required to complete it (exit code $Code)." }
            return
        }
        $hint = if ($Code -eq 1602 -or $Code -eq 1223) { ' (cancelled)' } elseif ($Code -eq 1925) { ' (needs administrator rights)' } else { '' }
        throw "$What failed with msiexec exit code $Code$hint. Log: $Log"
    }

    # ---- uninstall -------------------------------------------------------------------------------------
    $perUserDir = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'Programs\synofurb' } else { Join-Path ([IO.Path]::GetTempPath()) 'synofurb' }
    $portableDir = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'Programs\synofurb-portable' } else { Join-Path ([IO.Path]::GetTempPath()) 'synofurb-portable' }
    $startMenu = if ($env:APPDATA) { Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\synofurb.lnk' } else { '' }

    if ($Uninstall) {
        $did = $false
        foreach ($e in Get-InstalledEntries) {
            if ($e.Msi) {
                $code = $e.Name
                if ($code -notmatch '^\{[0-9A-Fa-f-]{36}\}$') { Write-Warning "skipping unexpected product code '$code'"; continue }
                Write-Host "Uninstalling the MSI product $code ..."
                if ($DryRun) { Write-Host "[dry-run] would run: msiexec /x $code /qn /norestart"; $did = $true; continue }
                $log = Join-Path ([IO.Path]::GetTempPath()) 'synofurb-uninstall.log'
                Assert-MsiOk (Invoke-Msiexec @('/x', $code, '/qn', '/norestart', '/L*v', $log)) 'uninstall' $log
                $did = $true
            }
        }
        $dirs = @($perUserDir, $portableDir)
        if ($Dir) { $dirs = @($Dir) }
        foreach ($d in $dirs) {
            if ((Test-Path -LiteralPath (Join-Path $d 'synofurb.exe')) -or ($TestMode -and (Test-Path -LiteralPath $d))) {
                if ($DryRun) { Write-Host "[dry-run] would remove $d"; $did = $true; continue }
                Remove-Item -LiteralPath $d -Recurse -Force
                Write-Host "Removed $d"
                $did = $true
            }
        }
        if (-not $DryRun) {
            if ($startMenu -and (Test-Path -LiteralPath $startMenu)) { Remove-Item -LiteralPath $startMenu -Force; Write-Host "Removed $startMenu" }
            if ($onWindows) {
                $key = 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\synofurb-user'
                if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key -Recurse -Force }
            }
        }
        if (-not $did) { Write-Host 'Nothing to remove: synofurb is not installed (looked for the MSI product and the per-user folders).' }
        else { Write-Host 'Your settings and inventory (%APPDATA%\synofurb) were left in place.' }
        return
    }

    # ---- plan ------------------------------------------------------------------------------------------
    $archRaw = if ($env:SYNOFURB_FORCE_ARCH) { $env:SYNOFURB_FORCE_ARCH } elseif ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } elseif ($env:PROCESSOR_ARCHITECTURE) { $env:PROCESSOR_ARCHITECTURE } else { [string][Runtime.InteropServices.RuntimeInformation]::OSArchitecture }
    switch -Regex ($archRaw) {
        '^(AMD64|x86_64|X64)$' { $arch = 'x86_64'; break }
        '^(ARM64|aarch64)$' { $arch = 'aarch64'; break }
        default { throw "unsupported CPU architecture: $archRaw (published Windows builds: x86_64, aarch64)" }
    }

    $versionPattern = '^[0-9]+\.[0-9]+\.[0-9]+([-+.][0-9A-Za-z.+-]*)?$'
    if ($Version) {
        $Version = $Version.TrimStart('v')
        if ($Version -notmatch $versionPattern) { throw "invalid -Version '$Version' (expected e.g. 0.2.0)" }
    } else {
        Write-Host 'Looking up the latest release ...'
        $rel = Get-Json "$ApiBase/releases/latest"
        $tagName = [string]$rel.tag_name
        $Version = $tagName.TrimStart('v')
        if ($Version -notmatch $versionPattern) { throw "refusing unexpected release tag from the API: $tagName" }
    }
    $tag = "v$Version"
    $usingZip = $Portable -or $User
    $ext = if ($usingZip) { 'zip' } else { 'msi' }
    $asset = "synofurb-$Version-windows-$arch.$ext"
    $urlBase = "$DlBase/$tag"
    if (-not $Dir) { $Dir = if ($User) { $perUserDir } elseif ($Portable) { $portableDir } else { '' } }
    if ($Dir -and -not [IO.Path]::IsPathRooted($Dir)) { throw "-Dir must be an absolute path (got: $Dir)" }
    if ($Dir -match '["\r\n]') { throw '-Dir must not contain double quotes or line breaks' }

    Write-Host "synofurb $Version for windows/$arch ($asset)"
    if ($DryRun) {
        Write-Host "[dry-run] would download $urlBase/$asset"
        Write-Host "[dry-run] would download $urlBase/SHA256SUMS and $urlBase/SHA256SUMS.minisig"
        if ($InsecureSkipSignature) { Write-Host '[dry-run] would verify SHA-256 only (-InsecureSkipSignature)' } else { Write-Host '[dry-run] would verify SHA-256 and, if minisign is installed, the signature' }
        if ($usingZip) { Write-Host "[dry-run] would extract to $Dir$(if ($User) { ' and add a Start Menu shortcut + Apps & features entry' })" }
        else { Write-Host "[dry-run] would run: msiexec /i $asset /qn /norestart$(if ($Dir) { ' INSTALLDIR="' + $Dir + '"' })  (elevating only msiexec if needed)" }
        if ($Run.Count -gt 0) { Write-Host "[dry-run] would run: synofurb $($Run -join ' ')" }
        Write-Host '[dry-run] nothing was downloaded or changed.'
        return
    }

    # ---- download + verify -----------------------------------------------------------------------------
    $work = Join-Path ([IO.Path]::GetTempPath()) ('synofurb-install-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work | Out-Null
    $exeToRun = $null
    try {
        Write-Host 'Downloading ...'
        $sumsPath = Join-Path $work 'SHA256SUMS'
        Save-File "$urlBase/SHA256SUMS" $sumsPath

        # signature of SHA256SUMS
        $signed = $false
        if ($InsecureSkipSignature) {
            Write-Warning 'signature verification SKIPPED (-InsecureSkipSignature): only the SHA-256 is checked, which cannot detect a tampered release.'
        } else {
            $ms = Get-Command $MinisignName -ErrorAction SilentlyContinue
            if (-not $ms) {
                $msg = "minisign is not installed, so the release signature was NOT verified (only the SHA-256 will be checked: integrity, not authenticity). Install it with 'scoop install minisign' or 'choco install minisign' (or from https://jedisct1.github.io/minisign/) and run this again to verify."
                if ($RequireSignature) { throw $msg }
                Write-Warning $msg
            } elseif ($PublicKey -eq 'REPLACE_WITH_MINISIGN_PUBLIC_KEY') {
                $msg = 'this installer has no minisign public key built in yet (development copy); pass -PublicKey KEY to verify the signature.'
                if ($RequireSignature) { throw $msg }
                Write-Warning $msg
            } else {
                $sigPath = Join-Path $work 'SHA256SUMS.minisig'
                Save-File "$urlBase/SHA256SUMS.minisig" $sigPath
                & $ms.Source -Vm $sumsPath -x $sigPath -P $PublicKey *> $null
                if ($LASTEXITCODE -ne 0) { throw 'SIGNATURE VERIFICATION FAILED for SHA256SUMS - the release is not authentic. Nothing was installed.' }
                Write-Host 'Signature OK (minisign).'
                $signed = $true
            }
        }
        if ($RequireSignature -and -not $signed) { throw '-RequireSignature was given but the signature could not be verified.' }

        $expected = $null
        foreach ($line in (Get-Content -LiteralPath $sumsPath)) {
            if ($line -match '^([0-9a-fA-F]{64})\s+\*?(.+?)\s*$' -and $Matches[2] -ceq $asset) { $expected = $Matches[1].ToLowerInvariant(); break }
        }
        if (-not $expected) { throw "release $tag has no asset for windows/$arch ($asset) listed in SHA256SUMS. Published builds: https://github.com/$Repo/releases/tag/$tag" }

        $assetPath = Join-Path $work $asset
        Save-File "$urlBase/$asset" $assetPath
        $actual = (Get-FileHash -LiteralPath $assetPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { throw "SHA-256 MISMATCH for $asset (expected $expected, got $actual). The download is corrupt or tampered with. Nothing was installed." }
        Write-Host "SHA-256 OK ($actual)."

        # ---- install -----------------------------------------------------------------------------------
        if (-not $usingZip) {
            $log = Join-Path ([IO.Path]::GetTempPath()) 'synofurb-install.log'
            $msiArgs = @('/i', $assetPath, '/qn', '/norestart', '/L*v', $log)
            if ($Dir) { $msiArgs += "INSTALLDIR=$Dir" }
            Write-Host 'Installing the MSI (silent) ...'
            Assert-MsiOk (Invoke-Msiexec $msiArgs) 'install' $log
            $exeToRun = Find-Exe $Dir
            Write-Host "Installed$(if ($exeToRun) { ': ' + $exeToRun })"
        } else {
            $staging = Join-Path $work 'extract'
            # inspect the entry names BEFORE extracting (zip-slip)
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $zip = [IO.Compression.ZipFile]::OpenRead($assetPath)
            try {
                foreach ($entry in $zip.Entries) {
                    if ($entry.FullName -match '(^|[\\/])\.\.([\\/]|$)' -or [IO.Path]::IsPathRooted($entry.FullName) -or $entry.FullName -match '^[A-Za-z]:') { throw "archive entry '$($entry.FullName)' escapes the install folder - refusing to extract" }
                }
            } finally { $zip.Dispose() }
            Expand-Archive -LiteralPath $assetPath -DestinationPath $staging -Force
            # refuse anything that escaped the staging folder (zip-slip) before copying
            $stagingFull = [IO.Path]::GetFullPath($staging).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
            foreach ($f in Get-ChildItem -LiteralPath $staging -Recurse -Force) {
                if (-not [IO.Path]::GetFullPath($f.FullName).StartsWith($stagingFull, [StringComparison]::OrdinalIgnoreCase)) { throw 'archive contains paths outside its folder - refusing to install' }
                if ($f.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'archive contains links - refusing to install' }
            }
            if (-not (Test-Path -LiteralPath (Join-Path $staging 'synofurb.exe') -PathType Leaf)) { throw "archive does not contain synofurb.exe at its top level" }
            New-Item -ItemType Directory -Path $Dir -Force | Out-Null
            Copy-Item -Path (Join-Path $staging '*') -Destination $Dir -Recurse -Force
            $exeToRun = Join-Path $Dir 'synofurb.exe'
            Write-Host "Extracted to $Dir"

            if ($User -and $onWindows) {
                try {
                    $shell = New-Object -ComObject WScript.Shell
                    $lnk = $shell.CreateShortcut($startMenu)
                    $lnk.TargetPath = $exeToRun
                    $lnk.WorkingDirectory = $Dir
                    $lnk.Description = 'Reinstall Synology NAS units with plain Debian'
                    $lnk.Save()
                    Write-Host "Start Menu shortcut: $startMenu"
                } catch { Write-Warning "could not create the Start Menu shortcut: $($_.Exception.Message)" }
                try {
                    $key = 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\synofurb-user'
                    New-Item -Path $key -Force | Out-Null
                    Set-ItemProperty -LiteralPath $key -Name DisplayName -Value 'synofurb'
                    Set-ItemProperty -LiteralPath $key -Name DisplayVersion -Value $Version
                    Set-ItemProperty -LiteralPath $key -Name Publisher -Value 'synofurb'
                    Set-ItemProperty -LiteralPath $key -Name InstallLocation -Value $Dir
                    Set-ItemProperty -LiteralPath $key -Name DisplayIcon -Value $exeToRun
                    Set-ItemProperty -LiteralPath $key -Name NoModify -Value 1 -Type DWord
                    Set-ItemProperty -LiteralPath $key -Name NoRepair -Value 1 -Type DWord
                    # self-contained uninstall (no remote code is run at uninstall time)
                    $q = { param($x) $x -replace "'", "''" }
                    $cmd = "Remove-Item -LiteralPath '$(& $q $Dir)' -Recurse -Force; Remove-Item -LiteralPath '$(& $q $startMenu)' -Force -ErrorAction SilentlyContinue; Remove-Item -LiteralPath '$(& $q $key)' -Recurse -Force"
                    Set-ItemProperty -LiteralPath $key -Name UninstallString -Value ('powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "' + ($cmd -replace '"', '\"') + '"')
                } catch { Write-Warning "could not register the uninstall entry: $($_.Exception.Message)" }
            }
            if ($AddToPath -and $onWindows) {
                $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
                $parts = @(); if ($userPath) { $parts = $userPath -split ';' | Where-Object { $_ } }
                if ($parts -notcontains $Dir) {
                    [Environment]::SetEnvironmentVariable('Path', (($parts + $Dir) -join ';'), 'User')
                    Write-Host "Added $Dir to your user PATH (new terminals pick it up)."
                }
            }
        }
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    Write-Host "Done.$(if ($exeToRun) { ' Run:  & "' + $exeToRun + '" --help' })"

    if ($Run.Count -gt 0) {
        if (-not $exeToRun) { throw 'cannot find the installed synofurb.exe to run (a restart or a new terminal may be needed for PATH)' }
        Write-Host "Running: synofurb $($Run -join ' ')"
        & $exeToRun @Run
        $global:LASTEXITCODE = $LASTEXITCODE
    }
}

Install-Synofurb -Version $Version -Portable $Portable.IsPresent -User $User.IsPresent -Dir $Dir -AddToPath $AddToPath.IsPresent -Run $Run `
    -Uninstall $Uninstall.IsPresent -DryRun $DryRun.IsPresent -InsecureSkipSignature $InsecureSkipSignature.IsPresent `
    -RequireSignature $RequireSignature.IsPresent -PublicKey $PublicKey
