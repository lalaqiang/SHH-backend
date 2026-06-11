#requires -Version 5.0
# Compare schemas between local and remote, generate ALTER TABLE statements
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Continue'

$localCs = "Server=127.0.0.1,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"
$remoteCs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"

function Run-Query($conn, [string]$sql) {
    $cmd = $conn.CreateCommand()
    $cmd.CommandTimeout = 300
    $cmd.CommandText = $sql
    $da = New-Object System.Data.SqlClient.SqlDataAdapter $cmd
    $ds = New-Object System.Data.DataSet
    [void]$da.Fill($ds)
    ,$ds.Tables[0]
}

$rc = New-Object System.Data.SqlClient.SqlConnection $remoteCs
$rc.Open()
$lc = New-Object System.Data.SqlClient.SqlConnection $localCs
$lc.Open()

# Get all user tables
$tableList = Run-Query $lc "SELECT s.name + '.' + t.name FROM sys.tables t INNER JOIN sys.schemas s ON t.schema_id = s.schema_id WHERE t.is_ms_shipped = 0 AND t.type = 'U' ORDER BY s.name, t.name"

$alterSb = New-Object System.Text.StringBuilder
[void]$alterSb.AppendLine("-- ALTER statements to add missing columns to local")
[void]$alterSb.AppendLine("USE [TestERP];")
[void]$alterSb.AppendLine("GO")
[void]$alterSb.AppendLine("")

$missingTotal = 0
$tablesAffected = 0
foreach ($r in $tableList.Rows) {
    $full = [string]$r[0]
    $parts = $full -split '\.'
    $schema = $parts[0]
    $name = $parts[1]

    # Get local column metadata
    $localCols = @{}
    $ldt = Run-Query $lc "SELECT c.name, st.name AS sys_type, c.max_length, c.precision, c.scale, c.is_nullable FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id INNER JOIN sys.types st ON c.system_type_id = st.user_type_id WHERE c.object_id = OBJECT_ID('$full')"
    foreach ($lr in $ldt.Rows) {
        $localCols[[string]$lr[0]] = @{
            Type = [string]$lr[1]
            MaxLength = [int]$lr[2]
            Precision = [int]$lr[3]
            Scale = [int]$lr[4]
            Nullable = [bool]$lr[5]
        }
    }

    # Get remote column metadata
    $remoteCols = @{}
    $rdt = Run-Query $rc "SELECT c.name, st.name AS sys_type, c.max_length, c.precision, c.scale, c.is_nullable, t.name AS udt FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id INNER JOIN sys.types st ON c.system_type_id = st.user_type_id WHERE c.object_id = OBJECT_ID('$full')"
    foreach ($rr in $rdt.Rows) {
        $remoteCols[[string]$rr[0]] = @{
            Type = [string]$rr[1]
            MaxLength = [int]$rr[2]
            Precision = [int]$rr[3]
            Scale = [int]$rr[4]
            Nullable = [bool]$rr[5]
            UDT = [string]$rr[6]
        }
    }

    $missing = @()
    foreach ($k in $remoteCols.Keys) {
        if (-not $localCols.ContainsKey($k)) { $missing += $k }
    }
    if ($missing.Count -gt 0) {
        $tablesAffected++
        $missingTotal += $missing.Count
        Write-Host "$full : $($missing -join ', ')"
        foreach ($cn in $missing) {
            $info = $remoteCols[$cn]
            $typeStr = switch ($info.Type) {
                'varchar'       { "varchar($(if($info.MaxLength -eq -1){'MAX'}else{$info.MaxLength}))" }
                'nvarchar'      { "nvarchar($(if($info.MaxLength -eq -1){'MAX'}else{$info.MaxLength/2}))" }
                'char'          { "char($($info.MaxLength))" }
                'nchar'         { "nchar($($info.MaxLength/2))" }
                'binary'        { "binary($($info.MaxLength))" }
                'varbinary'     { "varbinary($(if($info.MaxLength -eq -1){'MAX'}else{$info.MaxLength}))" }
                'decimal'       { "decimal($($info.Precision),$($info.Scale))" }
                'numeric'       { "numeric($($info.Precision),$($info.Scale))" }
                'datetime2'     { "datetime2($($info.Scale))" }
                'time'          { "time($($info.Scale))" }
                'datetimeoffset'{ "datetimeoffset($($info.Scale))" }
                default         { $info.Type }
            }
            $nullStr = if ($info.Nullable) { "NULL" } else { "NOT NULL" }
            [void]$alterSb.AppendLine("IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[$schema].[$name]') AND name = '$cn')")
            [void]$alterSb.AppendLine("    ALTER TABLE [$schema].[$name] ADD [$cn] $typeStr $nullStr;")
            [void]$alterSb.AppendLine("GO")
        }
    }
}
$lc.Close()
$rc.Close()

[System.IO.File]::WriteAllText('C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\03_alter.sql', $alterSb.ToString(), [System.Text.Encoding]::UTF8)
Write-Host ""
Write-Host ("Tables affected: {0}, Total missing columns: {1}" -f $tablesAffected, $missingTotal)
Write-Host "Written to 03_alter.sql"
