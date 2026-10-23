# KaedeNiwa SQLite 在线备份 / 恢复
#
# 备份：
#   pwsh -File scripts/backup-db.ps1 -Mode backup -Db C:\kaede\kaede.db
#
# 恢复（必须先停止服务；需要 -Force 才会真正覆盖）：
#   pwsh -File scripts/backup-db.ps1 -Mode restore -Db C:\kaede\kaede.db `
#       -Backup C:\kaede\backups\kaede-20261022-120000.db -Force
param(
    [ValidateSet('backup', 'restore')]
    [string]$Mode = 'backup',

    [Parameter(Mandatory = $true)]
    [string]$Db,

    [string]$OutDir,

    [string]$Backup,

    [switch]$Force
)

$ErrorActionPreference = 'Stop'

if (-not (Get-Command sqlite3 -ErrorAction SilentlyContinue)) {
    Write-Host '错误: 未找到 sqlite3 命令。' -ForegroundColor Red
    Write-Host '  Windows: winget install SQLite.SQLite   （或使用 Docker 内的 sqlite3）' -ForegroundColor Yellow
    Write-Host '  Debian/Ubuntu: sudo apt-get install -y sqlite3' -ForegroundColor Yellow
    exit 1
}

function Assert-Integrity([string]$Path) {
    $result = & sqlite3 $Path 'PRAGMA integrity_check;'
    if ($LASTEXITCODE -ne 0 -or ($result -notcontains 'ok')) {
        throw "integrity_check 失败: $Path`n$result"
    }
}

function Sql([string]$Text) {
    # 单引号转义，避免路径中的引号破坏 SQL 字面量
    return $Text.Replace("'", "''")
}

if ($Mode -eq 'backup') {
    if (-not (Test-Path -LiteralPath $Db)) {
        throw "数据库不存在: $Db"
    }
    if (-not $OutDir) {
        $OutDir = Join-Path (Split-Path -Parent $Db) 'backups'
    }
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $dest = Join-Path $OutDir "kaede-$stamp.db"

    Write-Host "备份 $Db -> $dest"
    & sqlite3 $Db ".backup '$(Sql $dest)'"
    if ($LASTEXITCODE -ne 0) { throw 'sqlite3 .backup 失败' }

    Assert-Integrity $dest
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $dest).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($dest))" | Set-Content -LiteralPath "$dest.sha256" -Encoding ascii

    $size = [math]::Round((Get-Item -LiteralPath $dest).Length / 1MB, 2)
    Write-Host "完成: $dest ($size MiB)" -ForegroundColor Green
    Write-Host "SHA256: $hash"
    exit 0
}

# ---- restore ----
if (-not $Backup) { throw '恢复模式需要 -Backup <备份文件路径>' }
if (-not (Test-Path -LiteralPath $Backup)) { throw "备份文件不存在: $Backup" }

# 有 .sha256 sidecar 时先校验，避免恢复被截断/损坏的备份。
$hashFile = "$Backup.sha256"
if (Test-Path -LiteralPath $hashFile) {
    $expected = ((Get-Content -LiteralPath $hashFile -Raw) -split '\s+')[0].Trim().ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Backup).Hash.ToLowerInvariant()
    if ($expected -and $actual -ne $expected) {
        throw "备份 SHA256 不匹配（expected=$expected actual=$actual），拒绝恢复"
    }
    Write-Host 'SHA256 校验通过' -ForegroundColor Green
}

Assert-Integrity $Backup

if (-not $Force) {
    Write-Host '未指定 -Force，仅做校验，不执行恢复。' -ForegroundColor Yellow
    Write-Host '确认服务已停止后，追加 -Force 重新执行。' -ForegroundColor Yellow
    exit 2
}

foreach ($suffix in @('-wal', '-shm')) {
    $side = "$Db$suffix"
    if (Test-Path -LiteralPath $side) { Remove-Item -LiteralPath $side -Force }
}

Write-Host "恢复 $Backup -> $Db"
& sqlite3 $Db ".restore '$(Sql $Backup)'"
if ($LASTEXITCODE -ne 0) { throw 'sqlite3 .restore 失败' }

foreach ($suffix in @('-wal', '-shm')) {
    $side = "$Db$suffix"
    if (Test-Path -LiteralPath $side) { Remove-Item -LiteralPath $side -Force }
}

Assert-Integrity $Db
Write-Host '恢复完成，integrity_check = ok' -ForegroundColor Green
