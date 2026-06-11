$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

$full = "dbo.tBas_Emp"
Write-Host "Testing $full ..."
$cmd = $rc.CreateCommand()
$cmd.CommandTimeout = 180
$cmd.CommandText = "SELECT TOP 3 * FROM [$full]"
try {
    $rdr = $cmd.ExecuteReader()
    $rows = 0
    while ($rdr.Read()) {
        $rows++
        for ($c = 0; $c -lt $rdr.FieldCount; $c++) {
            $v = $rdr.GetValue($c)
            if ($v -is [string]) {
                if ($v -match '\bif\b' -or $v -match '\bselect\b' -or $v -match '\bexec\b') {
                    Write-Host "  Row $rows Col $c has suspicious: [$v]"
                }
            }
        }
    }
    $rdr.Close()
    Write-Host "Read $rows rows successfully"
} catch {
    Write-Host "ERROR: $($_.Exception.Message)"
    Write-Host "TYPE: $($_.Exception.GetType().FullName)"
}
$rc.Close()
