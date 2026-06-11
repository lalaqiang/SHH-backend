# Try the exact data extraction logic from main script
$ErrorActionPreference = 'Continue'
$full = "dbo.tBas_Emp"
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

# Get column list with same query as main script
$rawC = Run-Query $rc "SELECT c.column_id, c.name, t.name, c.is_computed, c.is_identity, c.is_nullable FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id WHERE c.object_id = OBJECT_ID('$full') ORDER BY c.column_id"

Write-Host "rawC null: $($null -eq $rawC)"
Write-Host "rawC type: $($rawC.GetType().FullName)"
Write-Host "rows: $($rawC.Rows.Count)"

$colMap = @{}
$insertCols = @()
$idCols = @()
for ($i = 0; $i -lt $rawC.Rows.Count; $i++) {
    $items = $rawC.Rows[$i].ItemArray
    $cn    = [string]$items[1]
    $type  = [string]$items[2]
    $isComp = [bool]$items[3]
    $isId   = [bool]$items[4]
    $colMap[$cn] = @{ Type = $type; IsIdentity = $isId; IsComputed = $isComp }
    if (-not $isComp) { $insertCols += $cn }
    if ($isId) { $idCols += $cn }
}
Write-Host "insertCols count: $($insertCols.Count)"
Write-Host "idCols: $($idCols -join ', ')"

$colList = ($insertCols | ForEach-Object { "[$_]" }) -join ', '
Write-Host "colList length: $($colList.Length)"
Write-Host "colList first 100: $($colList.Substring(0, [Math]::Min(100, $colList.Length)))"

# Now try SELECT *
$parts = $full -split '\.'
$schema = $parts[0]
$name = $parts[1]
$selectSql = "SELECT TOP 1000 * FROM [$schema].[$name]"
Write-Host "SQL: $selectSql"

try {
    $cmd = $rc.CreateCommand()
    $cmd.CommandTimeout = 180
    $cmd.CommandText = $selectSql
    $rdr = $cmd.ExecuteReader()
    Write-Host "Reader opened, fields: $($rdr.FieldCount)"

    $rows = 0
    $insSb = New-Object System.Text.StringBuilder
    [void]$insSb.AppendLine("SET IDENTITY_INSERT [$schema].[$name] ON;")
    [void]$insSb.AppendLine("DELETE FROM [$schema].[$name];")

    while ($rdr.Read()) {
        $rows++
        $vals = @()
        for ($c = 0; $c -lt $rdr.FieldCount; $c++) {
            $rdrName = $rdr.GetName($c)
            if (-not $colMap.ContainsKey($rdrName) -or $colMap[$rdrName].IsComputed) { continue }
            $v = $rdr.GetValue($c)
            if ($v -is [DBNull]) {
                $vals += 'NULL'
            } elseif ($v -is [byte[]]) {
                $bytes = [byte[]]$v
                if ($bytes.Length -eq 0) { $vals += '0x' } else { $vals += ('0x' + [BitConverter]::ToString($bytes).Replace('-','')) }
            } elseif ($v -is [string]) {
                $s = $v -replace "'", "''"
                if ($s.Length -gt 4000) { $s = $s.Substring(0, 4000) }
                $vals += "N'$s'"
            } elseif ($v -is [DateTime]) {
                $vals += "'" + $v.ToString('yyyy-MM-dd HH:mm:ss.fff') + "'"
            } elseif ($v -is [bool]) {
                $vals += (if ($v) { 1 } else { 0 })
            } elseif ($v -is [decimal] -or $v -is [double] -or $v -is [single]) {
                $vals += $v.ToString([System.Globalization.CultureInfo]::InvariantCulture)
            } else {
                $s = $v.ToString().Replace("'", "''")
                $vals += "N'$s'"
            }
        }
        [void]$insSb.AppendLine("INSERT INTO [$schema].[$name] ($colList) VALUES ($($vals -join ', '));")
        if ($rows -ge 3) { break }
    }
    $rdr.Close()
    [void]$insSb.AppendLine("SET IDENTITY_INSERT [$schema].[$name] OFF;")
    Write-Host "Read $rows rows"
    Write-Host "Output sample:"
    $lines = $insSb.ToString() -split "`n"
    foreach ($l in ($lines | Select-Object -First 5)) { Write-Host $l }
} catch {
    Write-Host "ERROR: $($_.Exception.Message)"
    Write-Host "STACK: $($_.Exception.StackTrace)"
}
$rc.Close()
