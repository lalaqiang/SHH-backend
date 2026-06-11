$ErrorActionPreference = 'Continue'
$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

$cmd = $rc.CreateCommand()
$cmd.CommandText = "SELECT TOP 1 * FROM [dbo].[tBas_Emp]"
$rdr = $cmd.ExecuteReader()
$rdr.Read()

# Try to build the exact same string the main script builds
$schema = "dbo"
$name = "tBas_Emp"
$colList = "[EmpID], [EmpNo], [EmpName]"
$vals = @()

for ($c = 0; $c -lt 3; $c++) {
    $v = $rdr.GetValue($c)
    Write-Host ("Col {0}: type={1}" -f $c, $v.GetType().FullName)
    if ($v -is [DBNull]) {
        $vals += 'NULL'
    } elseif ($v -is [string]) {
        $s = $v -replace "'", "''"
        $vals += "N'$s'"
    } else {
        $s = $v.ToString().Replace("'", "''")
        $vals += "N'$s'"
    }
}

Write-Host "Vals: $($vals -join ', ')"
try {
    $s = "INSERT INTO [$schema].[$name] ($colList) VALUES ($($vals -join ', '));"
    Write-Host "OK: $s"
} catch {
    Write-Host "FAILED: $($_.Exception.Message)"
    Write-Host "STACK: $($_.Exception.StackTrace)"
}

$rdr.Close()
$rc.Close()
