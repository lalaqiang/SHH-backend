#requires -Version 5.0
# Test different DataRow access patterns
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

$cmd = $rc.CreateCommand()
$cmd.CommandText = "SELECT c.column_id, c.name, c.is_computed, c.is_identity FROM sys.columns c WHERE c.object_id = OBJECT_ID('dbo.brand') ORDER BY c.column_id"
$da = New-Object System.Data.SqlClient.SqlDataAdapter $cmd
$ds = New-Object System.Data.DataSet
[void]$da.Fill($ds)
$rawC = $ds.Tables[0]

Write-Host "Type of rawC: $($rawC.GetType().FullName)"
Write-Host "Rows.Count: $($rawC.Rows.Count)"

# Test 1: ItemArray on Row[0]
$r0 = $rawC.Rows[0]
Write-Host "Row[0] type: $($r0.GetType().FullName)"
Write-Host "Row[0] is null: $($null -eq $r0)"
$arr = $r0.ItemArray
Write-Host "ItemArray length: $($arr.Length)"
Write-Host "ItemArray[0]: $($arr[0])"
Write-Host "ItemArray[1]: $($arr[1])"
Write-Host "ItemArray[2]: $($arr[2])"
Write-Host "ItemArray[3]: $($arr[3])"

# Test 2: for-loop
Write-Host ""
Write-Host "For-loop test:"
for ($i = 0; $i -lt $rawC.Rows.Count; $i++) {
    $row = $rawC.Rows[$i]
    Write-Host "  i=$i  type=$($row.GetType().FullName)  null=$($null -eq $row)"
    $items = $row.ItemArray
    Write-Host "    ItemArray: $(($items | ForEach-Object { "$($_)" }) -join ' | ')"
}

$rc.Close()
