$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True;Application Name=copy_remote_db"
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
Write-Host "=== Simulating main script for $full ==="

# Get column metadata
$rawC = Run-Query $rc "SELECT c.column_id, c.name, t.name, c.is_computed, c.is_identity, c.is_nullable FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id WHERE c.object_id = OBJECT_ID('$full') ORDER BY c.column_id"
Write-Host "rawC type: $($rawC.GetType().FullName)"
Write-Host "rawC.Rows.Count: $($rawC.Rows.Count)"

$insertCols = @()
$idCols = @()
$colMap = @{}
$rc1 = $rawC.Rows.Count
for ($i = 0; $i -lt $rc1; $i++) {
    $items = $rawC.Rows[$i].ItemArray
    $cn    = [string]$items[1]
    $type  = [string]$items[2]
    $isComp = [bool]$items[3]
    $isId   = [bool]$items[4]
    Write-Host ("  Row {0}: cn=[{1}] type=[{2}] isComp={3} isId={4}" -f $i, $cn, $type, $isComp, $isId)
    $colMap[$cn] = @{ Type = $type; IsIdentity = $isId; IsComputed = $isComp }
    if (-not $isComp) { $insertCols += $cn }
    if ($isId) { $idCols += $cn }
}
Write-Host "insertCols: $($insertCols -join ' | ')"
Write-Host "colList: $(($insertCols | ForEach-Object { '['+$_+']' }) -join ', ')"

# Test SELECT *
$cmd = $rc.CreateCommand()
$cmd.CommandText = "SELECT TOP 3 * FROM [dbo].[brand]"
$rdr = $cmd.ExecuteReader()
Write-Host "Reader FieldCount: $($rdr.FieldCount)"
for ($j = 0; $j -lt $rdr.FieldCount; $j++) {
    Write-Host ("  Field {0}: name=[{1}]" -f $j, $rdr.GetName($j))
}
$rdr.Close()
$rc.Close()
