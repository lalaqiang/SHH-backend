$tables = @('dbo.tBas_Emp', 'dbo.tBas_EmpApply', 'dbo.tmp_tbas_Emp', 'dbo.tOA_LineMan', 'dbo.tSys_MD')
$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

foreach ($t in $tables) {
    $cmd = $rc.CreateCommand()
    $cmd.CommandText = "SELECT COUNT(*) FROM [$t]"
    try {
        $cnt = $cmd.ExecuteScalar()
        Write-Host "$t : $cnt rows"
    } catch {
        Write-Host "$t : ERROR - $($_.Exception.Message)"
    }
}

# Now check the actual row count of the last successful and the first failing
$cmd2 = $rc.CreateCommand()
$cmd2.CommandText = "SELECT s.name, t.name FROM sys.tables t INNER JOIN sys.schemas s ON t.schema_id = s.schema_id WHERE t.name IN ('tBas_Dept','tBas_Emp','tBas_EmpApply','tmp_tbas_Emp','tOA_LineMan','tSys_MD')"
$rdr = $cmd2.ExecuteReader()
Write-Host ""
Write-Host "Schema check:"
while ($rdr.Read()) {
    Write-Host "  $($rdr[0]).$($rdr[1])"
}
$rdr.Close()
$rc.Close()
