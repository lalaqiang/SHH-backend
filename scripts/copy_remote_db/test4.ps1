$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

# Test 1: SELECT *
Write-Host "Test 1: SELECT *"
$cmd = $rc.CreateCommand()
$cmd.CommandText = "SELECT TOP 3 * FROM [dbo].[brand]"
$da = New-Object System.Data.SqlClient.SqlDataAdapter $cmd
$ds = New-Object System.Data.DataSet
[void]$da.Fill($ds)
$dt = $ds.Tables[0]
Write-Host "Rows: $($dt.Rows.Count)  Cols: $($dt.Columns.Count)"
foreach ($c in $dt.Columns) { Write-Host "  Col: $($c.ColumnName) (type=$($c.DataType.Name))" }

# Test 2: Use column ordinal only via sys.columns query
Write-Host ""
Write-Host "Test 2: SELECT with explicit column list using literal"
$cmd2 = $rc.CreateCommand()
$cmd2.CommandText = "SELECT TOP 3 [商品品牌], [id] FROM [dbo].[brand]"
try {
    $da2 = New-Object System.Data.SqlClient.SqlDataAdapter $cmd2
    $ds2 = New-Object System.Data.DataSet
    [void]$da2.Fill($ds2)
    Write-Host "Rows: $($ds2.Tables[0].Rows.Count)"
} catch {
    Write-Host "ERROR: $($_.Exception.Message)"
}

# Test 3: Use N'' prefix
Write-Host ""
Write-Host "Test 3: Try with collation change"
$cmd3 = $rc.CreateCommand()
$cmd3.CommandText = "SELECT TOP 3 * FROM [dbo].[brand]"
$rdr = $cmd3.ExecuteReader()
$schema = $rdr.GetSchemaTable()
$rdr.Close()
foreach ($r in $schema.Rows) {
    Write-Host "  Col: $($r['ColumnName'])  Ordinal=$($r['ColumnOrdinal'])"
}

$rc.Close()
