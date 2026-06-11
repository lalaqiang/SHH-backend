#requires -Version 5.0
# Debug: test single table
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Continue'

$RemoteServer = 'shenhuahui.f3322.org,1433'
$RemoteDb     = 'TestERP'
$SqlUser      = 'sa'
$SqlPwd       = 'sa123456'

$cs = "Server=$RemoteServer;Database=$RemoteDb;User ID=$SqlUser;Password=$SqlPwd;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True;Application Name=copy_remote_db"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

function Run-Query($conn, [string]$sql) {
    $cmd = $conn.CreateCommand()
    $cmd.CommandTimeout = 300
    $cmd.CommandText = $sql
    $da = New-Object System.Data.SqlClient.SqlDataAdapter $cmd
    $ds = New-Object System.Data.DataSet
    [void]$da.Fill($ds)
    return $ds.Tables[0]
}

$full = "dbo.brand"
Write-Host "=== Test for $full ==="
$rawC = Run-Query $rc "SELECT c.column_id, c.name, c.is_computed, c.is_identity FROM sys.columns c WHERE c.object_id = OBJECT_ID('$full') ORDER BY c.column_id"
Write-Host "rawC is null: $($null -eq $rawC)"
Write-Host "rawC.Rows.Count: $($rawC.Rows.Count)"
foreach ($cr in $rawC.Rows) {
    Write-Host "  Row: type=$($cr.GetType().FullName)"
    if ($cr -eq $null) { Write-Host "  CR IS NULL!"; continue }
    Write-Host "  cr[0]=$($cr[0])  cr[1]=$($cr[1])  cr[2]=$($cr[2])  cr[3]=$($cr[3])"
}

# Test a problematic table - sheet
$full2 = "dbo.Sheet1`$"
Write-Host ""
Write-Host "=== Test for $full2 ==="
$rawC2 = Run-Query $rc "SELECT c.column_id, c.name, c.is_computed, c.is_identity FROM sys.columns c WHERE c.object_id = OBJECT_ID('$full2') ORDER BY c.column_id"
Write-Host "rawC2 is null: $($null -eq $rawC2)"
Write-Host "rawC2.Rows.Count: $($rawC2.Rows.Count)"
foreach ($cr in $rawC2.Rows) {
    Write-Host "  Row: type=$($cr.GetType().FullName)"
    if ($cr -eq $null) { Write-Host "  CR IS NULL!"; continue }
    Write-Host "  cr[0]=$($cr[0])  cr[1]=$($cr[1])  cr[2]=$($cr[2])  cr[3]=$($cr[3])"
}

$rc.Close()
