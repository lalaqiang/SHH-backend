#requires -Version 5.0
# For each table that has more than 1000 rows locally, trim down to 1000
# Uses TOP 1000 ORDER BY (SELECT NULL) to delete excess
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Continue'

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

$truncated = 0
$skipped = 0
$errored = 0
foreach ($r in $tableList.Rows) {
    $full = [string]$r[0]
    $parts = $full -split '\.'
    $schema = $parts[0]
    $name = $parts[1]

    # Get local count
    $cmd = $lc.CreateCommand()
    $cmd.CommandText = "SELECT COUNT(*) FROM [$schema].[$name]"
    $localCount = [int]$cmd.ExecuteScalar()

    # Get remote count
    $cmd2 = $rc.CreateCommand()
    $cmd2.CommandText = "SELECT COUNT(*) FROM [$schema].[$name]"
    $remoteCount = [int]$cmd2.ExecuteScalar()

    if ($localCount -le 1000) {
        $skipped++
        continue
    }
    if ($remoteCount -le 0) {
        $skipped++
        continue
    }

    # Local has more than 1000 - trim to 1000
    $targetCount = 1000
    Write-Host "Trimming $full : $localCount -> $targetCount (remote=$remoteCount)"

    try {
        # Use SET ROWCOUNT to limit the delete to the first (localCount - 1000) rows
        $delCount = $localCount - $targetCount
        $delCmd = $lc.CreateCommand()
        $delCmd.CommandTimeout = 600
        # Delete in batches to avoid log issues
        $delCmd.CommandText = "SET ROWCOUNT $delCount; DELETE FROM [$schema].[$name]; SET ROWCOUNT 0;"
        $affected = $delCmd.ExecuteNonQuery()
        Write-Host "  Deleted $affected rows"

        # Verify
        $cmd = $lc.CreateCommand()
        $cmd.CommandText = "SELECT COUNT(*) FROM [$schema].[$name]"
        $newCount = [int]$cmd.ExecuteScalar()
        Write-Host "  New count: $newCount"
        $truncated++
    } catch {
        Write-Host "  ERROR: $($_.Exception.Message)"
        $errored++
    }
}
$lc.Close()
$rc.Close()
Write-Host ""
Write-Host ("Truncated: {0}, Skipped: {1}, Errored: {2}" -f $truncated, $skipped, $errored)
