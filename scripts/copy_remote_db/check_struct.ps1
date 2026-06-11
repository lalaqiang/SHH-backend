$tables = @('dbo.tBas_Emp','dbo.tBas_EmpApply','dbo.tmp_tbas_Emp','dbo.tOA_LineMan','dbo.tSys_MD')
$conn = New-Object System.Data.SqlClient.SqlConnection "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True;Application Name=check_struct"
$conn.Open()
foreach ($t in $tables) {
    $sql = "SELECT TOP 1 * FROM $t"
    $cmd = $conn.CreateCommand()
    $cmd.CommandText = $sql
    $rdr = $cmd.ExecuteReader()
    $schema = $rdr.GetSchemaTable()
    Write-Host "=== $t ==="
    Write-Host ("  cols={0}" -f $schema.Rows.Count)
    foreach ($r in $schema.Rows) {
        Write-Host ("  {0}  {1}" -f $r.ColumnName, $r.DataTypeName)
    }
    $rdr.Close()
}
$conn.Close()
