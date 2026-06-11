$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

$cmd = $rc.CreateCommand()
$cmd.CommandText = "SELECT c.column_id, c.name, t.name, c.is_computed, c.is_identity, c.is_nullable FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id WHERE c.object_id = OBJECT_ID('dbo.brand') ORDER BY c.column_id"
$da = New-Object System.Data.SqlClient.SqlDataAdapter $cmd
$ds = New-Object System.Data.DataSet
[void]$da.Fill($ds)
$rawC = $ds.Tables[0]

Write-Host "Columns:"
for ($j = 0; $j -lt $rawC.Columns.Count; $j++) {
    Write-Host "  [$j] = $($rawC.Columns[$j].ColumnName)"
}
Write-Host "Rows: $($rawC.Rows.Count)"
for ($i = 0; $i -lt $rawC.Rows.Count; $i++) {
    $items = $rawC.Rows[$i].ItemArray
    Write-Host "  Row $i :"
    for ($k = 0; $k -lt $items.Length; $k++) {
        Write-Host "    [$k] = [$($items[$k])]"
    }
}
$rc.Close()
