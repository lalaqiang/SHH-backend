$ErrorActionPreference = 'Continue'
$full = "dbo.tBas_Emp"
$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

$cmd = $rc.CreateCommand()
$cmd.CommandText = "SELECT TOP 1 * FROM [dbo].[tBas_Emp]"
$rdr = $cmd.ExecuteReader()
$rdr.Read()

Write-Host "Reading all field values one by one..."
for ($c = 0; $c -lt $rdr.FieldCount; $c++) {
    $name = $rdr.GetName($c)
    try {
        $v = $rdr.GetValue($c)
        $tname = if ($v -is [DBNull]) { "NULL" } else { $v.GetType().Name }
        $s = if ($v -is [DBNull]) { "NULL" } else { $v.ToString() }
        $preview = $s.Substring(0, [Math]::Min(60, $s.Length))
        Write-Host ("  [{0}] {1} ({2}): {3}" -f $c, $name, $tname, $preview)
    } catch {
        Write-Host ("  [{0}] {1}: ERROR - {2}" -f $c, $name, $_.Exception.Message)
    }
}
$rdr.Close()
$rc.Close()
