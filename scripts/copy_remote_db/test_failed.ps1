$ErrorActionPreference = 'Continue'
$tables = @('dbo.tBas_Emp', 'dbo.tBas_EmpApply', 'dbo.tmp_tbas_Emp', 'dbo.tOA_LineMan', 'dbo.tSys_MD')
$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

foreach ($full in $tables) {
    $parts = $full -split '\.'
    $schema = $parts[0]
    $name = $parts[1]
    Write-Host "=== $full ===" -ForegroundColor Yellow

    # Get row count
    $cmdCnt = $rc.CreateCommand()
    $cmdCnt.CommandText = "SELECT COUNT(*) FROM [$schema].[$name]"
    try {
        $cnt = $cmdCnt.ExecuteScalar()
        Write-Host "Row count: $cnt"
    } catch {
        Write-Host "COUNT failed: $($_.Exception.Message)"
    }

    # Try SELECT *
    $cmd = $rc.CreateCommand()
    $cmd.CommandText = "SELECT TOP 1 * FROM [$schema].[$name]"
    try {
        $rdr = $cmd.ExecuteReader()
        $fldCnt = $rdr.FieldCount
        Write-Host "SELECT * works, fields=$fldCnt"
        $rdr.Close()
    } catch {
        Write-Host "SELECT * FAILED: $($_.Exception.Message)"
        continue
    }

    # Try with column list
    $cmd2 = $rc.CreateCommand()
    $cmdCol = $rc.CreateCommand()
    $cmdCol.CommandText = "SELECT c.name FROM sys.columns c WHERE c.object_id = OBJECT_ID('$full') AND c.is_computed = 0 ORDER BY c.column_id"
    $rdr2 = $cmdCol.ExecuteReader()
    $colList = @()
    while ($rdr2.Read()) { $colList += $rdr2[0] }
    $rdr2.Close()
    $colListStr = ($colList | ForEach-Object { "[$_]" }) -join ', '
    $cmd2.CommandText = "SELECT TOP 1 $colListStr FROM [$schema].[$name]"
    try {
        $rdr3 = $cmd2.ExecuteReader()
        $rdr3.Close()
        Write-Host "SELECT with explicit cols works"
    } catch {
        Write-Host "SELECT with explicit cols FAILED: $($_.Exception.Message)"
    }
}
$rc.Close()
