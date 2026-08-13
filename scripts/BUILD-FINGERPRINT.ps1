$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
$inputs = @(
    (Join-Path $projectRoot "src"),
    (Join-Path $projectRoot "src-tauri\src"),
    (Join-Path $projectRoot "src-tauri\icons"),
    (Join-Path $projectRoot "src-tauri\Cargo.toml"),
    (Join-Path $projectRoot "src-tauri\Cargo.lock"),
    (Join-Path $projectRoot "src-tauri\tauri.conf.json"),
    (Join-Path $projectRoot "src-tauri\build.rs")
)

$files = foreach ($inputPath in $inputs) {
    if (Test-Path -LiteralPath $inputPath -PathType Container) {
        Get-ChildItem -LiteralPath $inputPath -Recurse -File
    } elseif (Test-Path -LiteralPath $inputPath -PathType Leaf) {
        Get-Item -LiteralPath $inputPath
    }
}

$rootUri = New-Object Uri(($projectRoot.TrimEnd('\') + '\'))
$lines = foreach ($file in ($files | Sort-Object FullName)) {
    $relative = [Uri]::UnescapeDataString($rootUri.MakeRelativeUri((New-Object Uri($file.FullName))).ToString())
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$relative=$hash"
}
$lines += "rustc=$(& rustc --version)"
$lines += "tauri=$(& cargo tauri --version)"

$bytes = [Text.Encoding]::UTF8.GetBytes(($lines -join "`n"))
$sha = [Security.Cryptography.SHA256]::Create()
try {
    ($sha.ComputeHash($bytes) | ForEach-Object { $_.ToString("x2") }) -join ""
} finally {
    $sha.Dispose()
}
