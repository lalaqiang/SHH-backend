$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

$cmd = $rc.CreateCommand()
$cmd.CommandText = "SELECT TOP 3 [商品品牌], [id] FROM [dbo].[brand]"
$da = New-Object System.Data.SqlClient.SqlDataAdapter $cmd
$ds = New-Object System.Data.DataSet
[void]$da.Fill($ds)
$dt = $ds.Tables[0]
Write-Host "Rows: $($dt.Rows.Count)"
foreach ($r in $dt.Rows) {
    Write-Host "  Row: $([string]$r[0])  $([int]$r[1])"
}
$rc.Close()
