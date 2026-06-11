$localCs = "Server=127.0.0.1,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"
$remoteCs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"

function Run-Query($conn, [string]$sql) {
    $cmd = $conn.CreateCommand()
    $cmd.CommandText = $sql
    $da = New-Object System.Data.SqlClient.SqlDataAdapter $cmd
    $ds = New-Object System.Data.DataSet
    [void]$da.Fill($ds)
    ,$ds.Tables[0]
}

$tables = @('tBas_Emp','tBas_EmpApply','tmp_tbas_Emp','tOA_LineMan','tSys_MD')

foreach ($t in $tables) {
    $lc = New-Object System.Data.SqlClient.SqlConnection $localCs
    $lc.Open()
    $rc = New-Object System.Data.SqlClient.SqlConnection $remoteCs
    $rc.Open()

    $localCols = @{}
    $ldt = Run-Query $lc "SELECT COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME='$t'"
    foreach ($r in $ldt.Rows) { $localCols[[string]$r[0]] = $true }

    $remoteCols = @{}
    $rdt = Run-Query $rc "SELECT COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME='$t'"
    foreach ($r in $rdt.Rows) { $remoteCols[[string]$r[0]] = $true }

    Write-Host "=== $t ==="
    Write-Host ("Local cols: {0}  Remote cols: {1}" -f $ldt.Rows.Count, $rdt.Rows.Count)
    foreach ($k in $localCols.Keys) {
        if (-not $remoteCols.ContainsKey($k)) { Write-Host "  LocalOnly: $k" }
    }
    foreach ($k in $remoteCols.Keys) {
        if (-not $localCols.ContainsKey($k)) { Write-Host "  RemoteOnly: $k" }
    }
    $lc.Close()
    $rc.Close()
}
