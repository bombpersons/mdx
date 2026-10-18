# Install mdx from a GitHub release (Windows, x64):
#
#   powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/bombpersons/mdx/main/install.ps1 | iex"
#
# Settings (environment variables):
#   MDX_VERSION=v0.1.0      a specific release instead of the latest
#   MDX_INSTALL_DIR=DIR     where mdx.exe goes (default %LOCALAPPDATA%\mdx\bin, added to your PATH)
#   MDX_NO_MCP=1            don't register mdx with Claude Code, Codex and OpenCode
#   MDX_DOWNLOAD_BASE=URL   fetch the archive from here instead (a mirror)
#
# Running it again updates mdx. Nothing else is downloaded: ROMs are yours
# to supply.
$ErrorActionPreference = 'Stop'

$repo = 'bombpersons/mdx'
if (-not [Environment]::Is64BitOperatingSystem) { throw 'mdx install: only 64-bit Windows has a prebuilt binary' }

$base = if ($env:MDX_DOWNLOAD_BASE) { $env:MDX_DOWNLOAD_BASE } elseif ($env:MDX_VERSION) { "https://github.com/$repo/releases/download/$env:MDX_VERSION" } else { "https://github.com/$repo/releases/latest/download" }
$asset = 'mdx-x86_64-pc-windows-msvc.zip'
$dir = if ($env:MDX_INSTALL_DIR) { $env:MDX_INSTALL_DIR } else { Join-Path (Join-Path $env:LOCALAPPDATA 'mdx') 'bin' }

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("mdx-install-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $label = if ($env:MDX_VERSION) { $env:MDX_VERSION } else { 'latest release' }
    Write-Host "Downloading $asset ($label)"
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile (Join-Path $tmp $asset)
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset.sha256" -OutFile (Join-Path $tmp "$asset.sha256")
    $expected = ((Get-Content (Join-Path $tmp "$asset.sha256") -Raw).Trim() -split '\s+')[0].ToLower()
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $asset)).Hash.ToLower()
    if ($expected -ne $actual) { throw "mdx install: checksum mismatch for $asset" }

    Expand-Archive -Path (Join-Path $tmp $asset) -DestinationPath (Join-Path $tmp 'x')
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Copy-Item -Force (Join-Path $tmp 'x\mdx.exe') (Join-Path $dir 'mdx.exe')
} finally {
    Remove-Item -Recurse -Force $tmp
}
$exe = Join-Path $dir 'mdx.exe'
Write-Host "Installed $(& $exe --version) to $exe"

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not (($userPath -split ';') -contains $dir)) {
    [Environment]::SetEnvironmentVariable('Path', ($(if ($userPath) { "$userPath;" } else { '' }) + $dir), 'User')
    Write-Host "Added $dir to your user PATH (open a new terminal to use it)"
}

if (-not $env:MDX_NO_MCP) {
    Write-Host ''
    Write-Host 'Registering mdx as an MCP server (set MDX_NO_MCP=1 to skip):'
    & $exe setup mcp
    if ($LASTEXITCODE -ne 0) { Write-Host 'Some clients could not be set up; see above.' }
}

Write-Host ''
Write-Host 'Next:'
Write-Host '  mdx header --rom C:\path\to\game.md   # try it on a cartridge'
Write-Host '  mdx guide port                        # porting a game to native code'
Write-Host '  mdx setup mcp                         # register again after installing an agent client'
