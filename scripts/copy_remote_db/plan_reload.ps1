#requires -Version 5.0
# For each table that has different row count vs remote, or local > 1000, fix it
# by re-extracting from remote and bulk-loading
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Continue'

$dir = 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\failed_tables'
$tmpDir = 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\reload_tables'
if (-not (Test-Path $tmpDir)) { New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null }

$localCs = "Server=127.0.0.1,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"
$remoteCs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"

function Run-Query($conn, [string]$sql) {
    $cmd = $conn.CreateCommand()
    $cmd.CommandTimeout = 300
    $cmd.CommandText = $sql
    $da = New-Object System.Data.SqlClient.SqlDataAdapter $cmd
    $ds = New-Object System.Data.DataSet
    [void]$da.Fill($ds)
    ,$ds.Tables[0]
}

$rc = New-Object System.Data.SqlClient.SqlConnection $remoteCs
$rc.Open()
$lc = New-Object System.Data.SqlClient.SqlConnection $localCs
$lc.Open()

# Get all user tables
$tableList = Run-Query $lc "SELECT s.name + '.' + t.name FROM sys.tables t INNER JOIN sys.schemas s ON t.schema_id = s.schema_id WHERE t.is_ms_shipped = 0 AND t.type = 'U' ORDER BY s.name, t.name"

# We only care about tables that need fixing (local > 1000 OR diff vs first 1000 from remote)
# Strategy: just re-process ALL tables where local != min(1000, remote_count)
# Use bcp to export then bcp/bulk insert to import
$toProcess = @()
foreach ($r in $tableList.Rows) {
    $full = [string]$r[0]
    $parts = $full -split '\.'
    $schema = $parts[0]
    $name = $parts[1]

    $cmd = $lc.CreateCommand()
    $cmd.CommandText = "SELECT COUNT(*) FROM [$schema].[$name]"
    $localCount = [int]$cmd.ExecuteScalar()

    $cmd2 = $rc.CreateCommand()
    $cmd2.CommandText = "SELECT COUNT(*) FROM [$schema].[$name]"
    $remoteCount = [int]$cmd2.ExecuteScalar()

    $expected = [Math]::Min(1000, $remoteCount)

    # Need to process if:
    # - local has more than 1000 (cleanup leftover)
    # - local doesn't have 1000 (need to add or reset)
    if ($localCount -ne $expected) {
        $toProcess += [PSCustomObject]@{
            Full = $full
            Local = $localCount
            Remote = $remoteCount
            Expected = $expected
        }
    }
}
$rc.Close()
$lc.Close()

Write-Host ("Tables to process: {0}" -f $toProcess.Count)
$toProcess | Select-Object -First 30 | Format-Table -AutoSize

# Output the list
$toProcess | Export-Csv (Join-Path $tmpDir 'to_process.csv') -NoTypeInformation -Encoding UTF8
Write-Host "Saved list to to_process.csv"
