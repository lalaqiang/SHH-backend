#requires -Version 5.0
# Copy tSal_Inv and tSal_InvDetail from remote to local
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Continue'

$localCs = "Server=127.0.0.1,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"
$remoteCs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"

$tables = @('tSal_Inv', 'tSal_InvDetail')

$rc = New-Object System.Data.SqlClient.SqlConnection $remoteCs
$rc.Open()
$lc = New-Object System.Data.SqlClient.SqlConnection $localCs
$lc.Open()

foreach ($table in $tables) {
    Write-Host "=== Processing $table ===" -ForegroundColor Cyan
    
    $cmd = $rc.CreateCommand()
    $cmd.CommandTimeout = 600
    $cmd.CommandText = "SELECT COUNT(*) FROM [$table]"
    $remoteCount = $cmd.ExecuteScalar()
    Write-Host "  Remote rows: $remoteCount"
    
    if ($remoteCount -eq 0) {
        Write-Host "  Skipped (empty)" -ForegroundColor Yellow
        continue
    }
    
    $cmd2 = $lc.CreateCommand()
    $cmd2.CommandTimeout = 600
    $cmd2.CommandText = "DELETE FROM [$table]"
    $cmd2.ExecuteNonQuery() | Out-Null
    Write-Host "  Cleared local table"
    
    $cmd3 = $rc.CreateCommand()
    $cmd3.CommandTimeout = 1200
    $cmd3.CommandText = "SELECT * FROM [$table]"
    $reader = $cmd3.ExecuteReader()
    
    $bulkCopy = New-Object System.Data.SqlClient.SqlBulkCopy($lc)
    $bulkCopy.DestinationTableName = $table
    $bulkCopy.BatchSize = 5000
    $bulkCopy.BulkCopyTimeout = 1200
    
    try {
        $bulkCopy.WriteToServer($reader)
        $reader.Close()
        
        $cmd4 = $lc.CreateCommand()
        $cmd4.CommandTimeout = 600
        $cmd4.CommandText = "SELECT COUNT(*) FROM [$table]"
        $localCount = $cmd4.ExecuteScalar()
        Write-Host "  Copied: $localCount rows" -ForegroundColor Green
    } catch {
        Write-Host "  ERROR: $($_.Exception.Message)" -ForegroundColor Red
        $reader.Close()
    }
    $bulkCopy.Close()
}

$rc.Close()
$lc.Close()
Write-Host "`nDone!" -ForegroundColor Green