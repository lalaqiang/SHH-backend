$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

$parts = @('dbo', 'tBas_Emp')
$schema = $parts[0]
$name = $parts[1]
$full = $schema + '.' + $name

Write-Host "Full: $full"

# try without brackets first
$cmd1 = $rc.CreateCommand()
$cmd1.CommandText = "SELECT TOP 1 * FROM $schema.$name"
try {
    $rdr1 = $cmd1.ExecuteReader()
    $rdr1.Close()
    Write-Host "OK without brackets"
} catch {
    Write-Host "FAILED without brackets: $($_.Exception.Message)"
}

# with brackets
$cmd2 = $rc.CreateCommand()
$cmd2.CommandText = "SELECT TOP 1 * FROM [$schema].[$name]"
try {
    $rdr2 = $cmd2.ExecuteReader()
    $rdr2.Close()
    Write-Host "OK with brackets"
} catch {
    Write-Host "FAILED with brackets: $($_.Exception.Message)"
}

# explicit full string with brackets
$cmd3 = $rc.CreateCommand()
$cmd3.CommandText = "SELECT TOP 1 * FROM [dbo].[tBas_Emp]"
try {
    $rdr3 = $cmd3.ExecuteReader()
    $rdr3.Close()
    Write-Host "OK with explicit brackets"
} catch {
    Write-Host "FAILED with explicit brackets: $($_.Exception.Message)"
}

# try just schema
$cmd4 = $rc.CreateCommand()
$cmd4.CommandText = "SELECT TOP 1 * FROM dbo.tBas_Emp"
try {
    $rdr4 = $cmd4.ExecuteReader()
    $rdr4.Close()
    Write-Host "OK without any brackets"
} catch {
    Write-Host "FAILED without any brackets: $($_.Exception.Message)"
}

$rc.Close()
