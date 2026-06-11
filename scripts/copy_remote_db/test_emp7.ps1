$ErrorActionPreference = 'Continue'
$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

function Run-Query($conn, [string]$sql) {
    $cmd = $conn.CreateCommand()
    $cmd.CommandTimeout = 300
    $cmd.CommandText = $sql
    $da = New-Object System.Data.SqlClient.SqlDataAdapter $cmd
    $ds = New-Object System.Data.DataSet
    [void]$da.Fill($ds)
    $dt = $ds.Tables[0]
    ,$dt
}

$full = "dbo.tBas_Emp"
$rawC = Run-Query $rc "SELECT c.column_id, c.name, t.name, c.is_computed, c.is_identity, c.is_nullable FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id WHERE c.object_id = OBJECT_ID('$full') ORDER BY c.column_id"

$colMap = @{}
for ($i = 0; $i -lt $rawC.Rows.Count; $i++) {
    $items = $rawC.Rows[$i].ItemArray
    $cn    = [string]$items[1]
    $type  = [string]$items[2]
    $isComp = [bool]$items[3]
    $isId   = [bool]$items[4]
    $colMap[$cn] = @{ Type = $type; IsIdentity = $isId; IsComputed = $isComp }
}

$cmd = $rc.CreateCommand()
$cmd.CommandText = "SELECT TOP 1 * FROM [dbo].[tBas_Emp]"
$rdr = $cmd.ExecuteReader()
$rdr.Read()

$vals = @()
for ($c = 0; $c -lt $rdr.FieldCount; $c++) {
    $rdrName = $rdr.GetName($c)
    if (-not $colMap.ContainsKey($rdrName)) {
        Write-Host "[$c] $rdrName NOT IN COLMAP"
        continue
    }
    if ($colMap[$rdrName].IsComputed) {
        Write-Host "[$c] $rdrName IS COMPUTED"
        continue
    }
    try {
        $v = $rdr.GetValue($c)
        $vals += "N'TEST'"
        Write-Host "[$c] $rdrName OK"
    } catch {
        Write-Host "[$c] $rdrName ERROR: $($_.Exception.Message)"
    }
}
$rdr.Close()
$rc.Close()
Write-Host "Done"
