$full = "dbo.tBas_Emp"
Write-Host "Test 1: [\$full]"
$s1 = "[$full]"
Write-Host "  Result: '$s1'"
Write-Host "  Length: $($s1.Length)"

Write-Host "Test 2: with type bracket"
$s2 = "[dbo].[tBas_Emp]"
Write-Host "  Result: '$s2'"

Write-Host "Test 3: escaped brackets"
$s3 = '[' + $full + ']'
Write-Host "  Result: '$s3'"

# test in SQL
$cs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True"
$rc = New-Object System.Data.SqlClient.SqlConnection $cs
$rc.Open()

$cmd1 = $rc.CreateCommand()
$cmd1.CommandText = "SELECT TOP 1 * FROM $s1"
try {
    $rdr = $cmd1.ExecuteReader()
    $rdr.Close()
    Write-Host "Test 1 SQL: OK"
} catch {
    Write-Host "Test 1 SQL FAILED: $($_.Exception.Message)"
    Write-Host "  SQL was: $($cmd1.CommandText)"
}

$cmd2 = $rc.CreateCommand()
$cmd2.CommandText = "SELECT TOP 1 * FROM $s3"
try {
    $rdr = $cmd2.ExecuteReader()
    $rdr.Close()
    Write-Host "Test 3 SQL: OK"
} catch {
    Write-Host "Test 3 SQL FAILED: $($_.Exception.Message)"
    Write-Host "  SQL was: $($cmd2.CommandText)"
}

$rc.Close()
