[CmdletBinding()]
param(
    [switch]$WriteGenerated
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
$mapsDirectory = Join-Path $projectRoot "src\assets\maps"
$manifestPath = Join-Path $mapsDirectory "manifest.json"
$rendererPath = Join-Path $projectRoot "src\map-control-replay.js"
$readmePath = Join-Path $mapsDirectory "README.txt"
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Stop-MapAssetVerification([string]$Message) {
    throw "Replay map asset verification failed: $Message"
}

function Normalize-Newlines([string]$Text) {
    return $Text.Replace("`r`n", "`n").Replace("`r", "`n")
}

function ConvertTo-JavaScriptString([string]$Value) {
    $escaped = $Value.Replace('\', '\\').Replace("'", "\'")
    $escaped = $escaped.Replace("`r", '\r').Replace("`n", '\n')
    return "'$escaped'"
}

if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    Stop-MapAssetVerification "missing canonical manifest src/assets/maps/manifest.json"
}

try {
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
} catch {
    Stop-MapAssetVerification "manifest.json is invalid JSON: $($_.Exception.Message)"
}

if ([int]$manifest.schema_version -ne 1) {
    Stop-MapAssetVerification "unsupported manifest schema_version '$($manifest.schema_version)'"
}

$version = [string]$manifest.ddragon_version
if ($version -notmatch '^\d+\.\d+\.\d+$') {
    Stop-MapAssetVerification "ddragon_version '$version' is not a complete Data Dragon version"
}

try {
    [DateTime]::ParseExact(
        [string]$manifest.fetched_on,
        'yyyy-MM-dd',
        [Globalization.CultureInfo]::InvariantCulture
    ) | Out-Null
} catch {
    Stop-MapAssetVerification "fetched_on must use YYYY-MM-DD"
}

$maps = @($manifest.maps | Sort-Object { [int]$_.map_id })
if ($maps.Count -eq 0) {
    Stop-MapAssetVerification "manifest contains no maps"
}

$seenIds = @{}
$expectedPngNames = @()
$pngSignature = '89504e470d0a1a0a'
foreach ($map in $maps) {
    $mapId = [int]$map.map_id
    if ($mapId -le 0 -or $seenIds.ContainsKey($mapId)) {
        Stop-MapAssetVerification "map IDs must be unique positive integers (found '$mapId')"
    }
    $seenIds[$mapId] = $true

    $name = [string]$map.name
    if ([string]::IsNullOrWhiteSpace($name)) {
        Stop-MapAssetVerification "map $mapId has no display name"
    }

    $filename = [string]$map.filename
    $expectedFilename = "map$mapId-$version.png"
    if ($filename -cne $expectedFilename) {
        Stop-MapAssetVerification "map $mapId filename '$filename' must be '$expectedFilename'"
    }
    if ([IO.Path]::GetFileName($filename) -cne $filename) {
        Stop-MapAssetVerification "map $mapId filename must not contain a directory"
    }

    $expectedSource = "https://ddragon.leagueoflegends.com/cdn/$version/img/map/map$mapId.png"
    if ([string]$map.source_url -cne $expectedSource) {
        Stop-MapAssetVerification "map $mapId source_url must be '$expectedSource'"
    }

    $expectedBytes = [long]$map.bytes
    if ($expectedBytes -le 8) {
        Stop-MapAssetVerification "map $mapId byte count must be greater than the PNG header"
    }
    $expectedHash = ([string]$map.sha256).ToLowerInvariant()
    if ($expectedHash -notmatch '^[0-9a-f]{64}$') {
        Stop-MapAssetVerification "map $mapId has an invalid SHA-256 digest"
    }

    $assetPath = Join-Path $mapsDirectory $filename
    if (-not (Test-Path -LiteralPath $assetPath -PathType Leaf)) {
        Stop-MapAssetVerification "missing pinned asset src/assets/maps/$filename"
    }
    $asset = Get-Item -LiteralPath $assetPath
    if ($asset.Length -ne $expectedBytes) {
        Stop-MapAssetVerification "map $mapId byte count is $($asset.Length), expected $expectedBytes"
    }
    $actualHash = (Get-FileHash -LiteralPath $assetPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -cne $expectedHash) {
        Stop-MapAssetVerification "map $mapId hash is $actualHash, expected $expectedHash"
    }

    $stream = [IO.File]::OpenRead($assetPath)
    try {
        $header = New-Object byte[] 8
        if ($stream.Read($header, 0, $header.Length) -ne $header.Length) {
            Stop-MapAssetVerification "map $mapId is too short to be a PNG"
        }
        $actualSignature = ($header | ForEach-Object { $_.ToString('x2') }) -join ''
        if ($actualSignature -cne $pngSignature) {
            Stop-MapAssetVerification "map $mapId does not have a PNG signature"
        }
    } finally {
        $stream.Dispose()
    }

    $expectedPngNames += $filename
}

$actualPngNames = @(Get-ChildItem -LiteralPath $mapsDirectory -File -Filter '*.png' |
    ForEach-Object { $_.Name } |
    Sort-Object)
$expectedPngNames = @($expectedPngNames | Sort-Object)
if (($actualPngNames -join "`n") -cne ($expectedPngNames -join "`n")) {
    Stop-MapAssetVerification (
        "PNG asset set differs from manifest. Expected [$($expectedPngNames -join ', ')], " +
        "found [$($actualPngNames -join ', ')]"
    )
}

$startMarker = '// BEGIN GENERATED MAP ASSET MANIFEST - DO NOT EDIT'
$endMarker = '// END GENERATED MAP ASSET MANIFEST'
$runtimeLines = @(
    $startMarker,
    'const MAP_ASSET_MANIFEST = Object.freeze({',
    "  ddragonVersion: $(ConvertTo-JavaScriptString $version),",
    '  maps: Object.freeze({'
)
foreach ($map in $maps) {
    $runtimeLines += "    $([int]$map.map_id): Object.freeze({ name: $(ConvertTo-JavaScriptString ([string]$map.name)), filename: $(ConvertTo-JavaScriptString ([string]$map.filename)) }),"
}
$runtimeLines += @(
    '  }),',
    '});',
    $endMarker
)
$expectedRuntimeBlock = $runtimeLines -join "`n"

$readmeLines = @(
    'Bundled replay map assets',
    '==========================',
    '',
    'THIS FILE IS GENERATED FROM manifest.json. Do not edit it directly.',
    '',
    'Aura bundles pinned official Data Dragon minimap images so post-match replay',
    'never downloads terrain during gameplay. The image number follows Riot''s map',
    "ID convention. Data Dragon version: $version.",
    ''
)
foreach ($map in $maps) {
    $readmeLines += @(
        [string]$map.filename,
        "  Map: $([int]$map.map_id) - $([string]$map.name)",
        "  Source: $([string]$map.source_url)",
        "  Bytes: $([long]$map.bytes)",
        "  SHA-256: $(([string]$map.sha256).ToLowerInvariant())",
        ''
    )
}
$readmeLines += @(
    "The bundled files are byte-identical to those URLs as fetched on $([string]$manifest.fetched_on).",
    'They are visual reference terrain only. They are not walkability, brush, or',
    'vision polygons, and Aura must not infer exact bush occupancy from the raster.',
    '',
    'League of Legends map artwork is owned by Riot Games and is used under Riot''s',
    'Legal Jibber Jabber policy: https://www.riotgames.com/en/legal'
)
$expectedReadme = ($readmeLines -join "`n") + "`n"

if (-not (Test-Path -LiteralPath $rendererPath -PathType Leaf)) {
    Stop-MapAssetVerification "missing src/map-control-replay.js"
}
$renderer = Normalize-Newlines (Get-Content -LiteralPath $rendererPath -Raw -Encoding UTF8)
$blockPattern = '(?ms)^' + [regex]::Escape($startMarker) + '\n.*?^' + [regex]::Escape($endMarker) + '$'
$blockMatches = [regex]::Matches($renderer, $blockPattern)
if ($blockMatches.Count -ne 1) {
    Stop-MapAssetVerification "renderer must contain exactly one generated map manifest block"
}

if ($WriteGenerated) {
    $updatedRenderer = [regex]::Replace($renderer, $blockPattern, $expectedRuntimeBlock, 1)
    [IO.File]::WriteAllText($rendererPath, $updatedRenderer.Replace("`n", "`r`n"), $utf8NoBom)
    [IO.File]::WriteAllText($readmePath, $expectedReadme.Replace("`n", "`r`n"), $utf8NoBom)
} else {
    if ($blockMatches[0].Value -cne $expectedRuntimeBlock) {
        Stop-MapAssetVerification "generated renderer metadata is stale; run scripts\VERIFY-MAP-ASSETS.ps1 -WriteGenerated"
    }
    if (-not (Test-Path -LiteralPath $readmePath -PathType Leaf)) {
        Stop-MapAssetVerification "missing generated src/assets/maps/README.txt"
    }
    $actualReadme = Normalize-Newlines (Get-Content -LiteralPath $readmePath -Raw -Encoding UTF8)
    if ($actualReadme -cne $expectedReadme) {
        Stop-MapAssetVerification "generated README is stale; run scripts\VERIFY-MAP-ASSETS.ps1 -WriteGenerated"
    }
}

Write-Host "[OK] Verified $($maps.Count) pinned replay map assets for Data Dragon $version."
