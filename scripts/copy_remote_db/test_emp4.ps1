$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

$cmd = $rc.CreateCommand()
$cmd.CommandTimeout = 60
$cmd.CommandText = "SELECT TOP 1 * FROM [dbo].[tBas_Emp]"
try {
    $rdr = $cmd.ExecuteReader()
    $rows = 0
    while ($rdr.Read()) {
        $rows++
        Write-Host ("Row {0}: FieldCount = {1}" -f $rows, $rdr.FieldCount)
        for ($c = 0; $c -lt $rdr.FieldCount; $c++) {
            try {
                $name = $rdr.GetName($c)
                $v = $rdr.GetValue($c)
                $t = if ($v -is [DBNull]) { "NULL" } else { $v.GetType().Name }
                Write-Host ("  [{0}] {1} = {2}" -f $c, $name, $t)
            } catch {
                Write-Host ("  [{0}] FAILED: {1}" -f $c, $_.Exception.Message)
            }
        }
    }
    $rdr.Close()
    Write-Host "Total rows: $rows"
} catch {
    Write-Host "ERROR: $($_.Exception.Message)"
    Write-Host "STACK: $($_.Exception.StackTrace)"
}
$rc.Close()
