#requires -Version 5.0
# Copy remote DB schema and top-1000 data of each table to local
# Uses SELECT * to avoid Chinese column name encoding issues on remote server
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Continue'

$RemoteServer = 'shenhuahui.f3322.org,1433'
$RemoteDb     = 'TestERP'
$LocalServer  = '127.0.0.1,1433'
$LocalDb      = 'TestERP'
$SqlUser      = 'sa'
$SqlPwd       = 'sa123456'

$OutDir = 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db'
if (!(Test-Path $OutDir)) { New-Item -ItemType Directory -Path $OutDir -Force | Out-Null }

$logFile = Join-Path $OutDir 'progress.log'
"" | Out-File $logFile -Encoding UTF8
function Log($msg) {
    $line = "[{0}] {1}" -f (Get-Date -Format 'HH:mm:ss'), $msg
    Write-Host $line
    Add-Content -Path $logFile -Value $line -Encoding UTF8
}

# ---------- helpers ----------
function New-Conn([string]$server, [string]$db) {
    $cs = "Server=$server;Database=$db;User ID=$SqlUser;Password=$SqlPwd;Connect Timeout=60;Encrypt=False;TrustServerCertificate=True;Application Name=copy_remote_db"
    $c = New-Object System.Data.SqlClient.SqlConnection $cs
    $c.Open()
    return $c
}

function Run-Query($conn, [string]$sql) {
    $cmd = $conn.CreateCommand()
    $cmd.CommandTimeout = 300
    $cmd.CommandText = $sql
    $da = New-Object System.Data.SqlClient.SqlDataAdapter $cmd
    $ds = New-Object System.Data.DataSet
    [void]$da.Fill($ds)
    $dt = $ds.Tables[0]
    ,$dt   # use unary comma to prevent PowerShell from unwrapping the DataTable
}

# ---------- 1) Load table list from files ----------
Log "Loading table lists from files ..."
$remoteTableFile = Join-Path $OutDir 'remote_tables.txt'
$localTableFile  = Join-Path $OutDir 'local_tables.txt'

$remoteList = Get-Content $remoteTableFile | ForEach-Object { $_.Trim() } | Where-Object { $_ }
$localSet = @{}
Get-Content $localTableFile | ForEach-Object { $_.Trim() } | Where-Object { $_ } | ForEach-Object { $localSet[$_] = $true }

Log "Remote tables: $($remoteList.Count)  Local tables: $($localSet.Count)"

# ---------- 2) DDL for missing tables ----------
$toCreate = $remoteList | Where-Object { -not $localSet.ContainsKey($_) }
Log "Tables to CREATE in local: $($toCreate.Count)"
foreach ($k in $toCreate) { Log "  + $k" }

Log "Connecting to remote for DDL extraction ..."
$rc = New-Conn $RemoteServer $RemoteDb

$ddlFile = Join-Path $OutDir '01_schema.sql'
$ddlLog  = Join-Path $OutDir '01_schema.log'
$ddlSb = New-Object System.Text.StringBuilder
[void]$ddlSb.AppendLine("-- DDL for tables missing in local")
[void]$ddlSb.AppendLine("-- Generated: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
[void]$ddlSb.AppendLine("USE [$LocalDb];")
[void]$ddlSb.AppendLine("GO")
[void]$ddlSb.AppendLine("SET QUOTED_IDENTIFIER ON; SET ANSI_NULLS ON;")
[void]$ddlSb.AppendLine("GO")
[void]$ddlSb.AppendLine("")

$ddlLogLines = @()
$idx = 0
foreach ($full in $toCreate) {
    $idx++
    $parts = $full -split '\.'
    $schema = $parts[0]
    $name   = $parts[1]
    Write-Progress -Activity "Generating DDL" -Status "$full ($idx/$($toCreate.Count))" -PercentComplete ([math]::Round($idx * 100 / [Math]::Max($toCreate.Count,1), 1))

    $rawCols = Run-Query $rc @"
SELECT c.column_id, c.name, t.name, c.max_length, c.precision, c.scale, c.is_nullable, c.is_identity, c.is_computed,
       ISNULL(ic.seed_value,0), ISNULL(ic.increment_value,0)
FROM sys.columns c
INNER JOIN sys.types t ON c.user_type_id = t.user_type_id
LEFT JOIN sys.identity_columns ic ON c.object_id = ic.object_id AND c.column_id = ic.column_id
WHERE c.object_id = OBJECT_ID('$full')
ORDER BY c.column_id
"@
    if ($null -eq $rawCols -or $rawCols.Rows.Count -eq 0) { $ddlLogLines += "WARN $full no columns"; continue }

    [void]$ddlSb.AppendLine("IF OBJECT_ID('[$schema].[$name]','U') IS NULL")
    [void]$ddlSb.AppendLine("BEGIN")
    [void]$ddlSb.AppendLine("CREATE TABLE [$schema].[$name] (")

    $colDefs = @()
    $pkCols = @()
    $rowCount = $rawCols.Rows.Count
    for ($i = 0; $i -lt $rowCount; $i++) {
        $items = $rawCols.Rows[$i].ItemArray
        $colName = [string]$items[1]
        $type    = [string]$items[2]
        $len     = [int]$items[3]
        $prec    = [int]$items[4]
        $scale   = [int]$items[5]
        $isNull  = [bool]$items[6]
        $isId    = [bool]$items[7]
        $isComp  = [bool]$items[8]
        $seed    = [string]$items[9]
        $inc     = [string]$items[10]

        $typeStr = switch ($type) {
            'varchar'       { "varchar($(if($len=-1){'MAX'}else{$len}))" }
            'nvarchar'      { "nvarchar($(if($len=-1){'MAX'}else{$len/2}))" }
            'char'          { "char($len)" }
            'nchar'         { "nchar($($len/2))" }
            'binary'        { "binary($len)" }
            'varbinary'     { "varbinary($(if($len=-1){'MAX'}else{$len}))" }
            'decimal'       { "decimal($prec,$scale)" }
            'numeric'       { "numeric($prec,$scale)" }
            'datetime2'     { "datetime2($scale)" }
            'time'          { "time($scale)" }
            'datetimeoffset'{ "datetimeoffset($scale)" }
            default         { $type }
        }
        $line = "    [$colName] $typeStr"
        if ($isId) { $line += " IDENTITY($seed,$inc)" }
        if (-not $isNull) { $line += " NOT NULL" }
        $colDefs += $line
        if (-not $isComp) {
            $colId = [int]$items[0]
            $rawPk = Run-Query $rc "SELECT ic.key_ordinal FROM sys.indexes i INNER JOIN sys.index_columns ic ON i.object_id=ic.object_id AND i.index_id=ic.index_id WHERE i.object_id=OBJECT_ID('$full') AND i.is_primary_key=1 AND ic.column_id=$colId"
            if ($null -ne $rawPk -and $rawPk.Rows.Count -gt 0) { $pkCols += "[$colName]" }
        }
    }
    [void]$ddlSb.AppendLine(($colDefs -join ",`n"))
    if ($pkCols.Count -gt 0) {
        [void]$ddlSb.AppendLine("    ,CONSTRAINT [PK_$name] PRIMARY KEY CLUSTERED ($($pkCols -join ', '))")
    }
    [void]$ddlSb.AppendLine(");")
    [void]$ddlSb.AppendLine("END")
    [void]$ddlSb.AppendLine("GO")
    [void]$ddlSb.AppendLine("")

    $ddlLogLines += "OK   $full  cols=$rowCount pk=$($pkCols.Count)"
}
[System.IO.File]::WriteAllText($ddlFile, $ddlSb.ToString(), [System.Text.Encoding]::UTF8)
[System.IO.File]::WriteAllText($ddlLog, ($ddlLogLines -join "`r`n"), [System.Text.Encoding]::UTF8)
Log "DDL written: $ddlFile  (tables=$($ddlLogLines.Count))"

# ---------- 3) Generate INSERT scripts using SELECT * (avoids encoding issue) ----------
$dataFile = Join-Path $OutDir '02_data.sql'
$dataLog  = Join-Path $OutDir '02_data.log'
$dataLogLines = @()
$dataSb = New-Object System.Text.StringBuilder
[void]$dataSb.AppendLine("-- Data: top 1000 rows per table from remote")
[void]$dataSb.AppendLine("-- Generated: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
[void]$dataSb.AppendLine("USE [$LocalDb];")
[void]$dataSb.AppendLine("GO")
[void]$dataSb.AppendLine("SET QUOTED_IDENTIFIER ON; SET ANSI_NULLS ON;")
[void]$dataSb.AppendLine("GO")
[void]$dataSb.AppendLine("")
[void]$dataSb.AppendLine("EXEC sp_msforeachtable 'ALTER TABLE ? NOCHECK CONSTRAINT ALL';")
[void]$dataSb.AppendLine("GO")
[void]$dataSb.AppendLine("")

$idx = 0
foreach ($full in $remoteList) {
    $idx++
    $parts = $full -split '\.'
    $schema = $parts[0]
    $name   = $parts[1]
    Write-Progress -Activity "Generating INSERTs" -Status "$full ($idx/$($remoteList.Count))" -PercentComplete ([math]::Round($idx * 100 / $remoteList.Count, 1))

    # Get column metadata (names, types, is_identity) from sys.columns
    $rawC = Run-Query $rc "SELECT c.column_id, c.name, t.name, c.is_computed, c.is_identity, c.is_nullable FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id WHERE c.object_id = OBJECT_ID('$full') ORDER BY c.column_id"
    if ($null -eq $rawC) { $dataLogLines += "SKIP $full  column query failed"; continue }

    # Build mapping: column name -> {is_computed, is_identity, is_nullable, type}
    $colMap = @{}
    $insertCols = @()
    $idCols = @()
    $rc1 = $rawC.Rows.Count
    for ($i = 0; $i -lt $rc1; $i++) {
        $items = $rawC.Rows[$i].ItemArray
        $cn    = [string]$items[1]
        $type  = [string]$items[2]
        $isComp = [bool]$items[3]
        $isId   = [bool]$items[4]
        $colMap[$cn] = @{ Type = $type; IsIdentity = $isId; IsComputed = $isComp }
        if (-not $isComp) { $insertCols += $cn }
        if ($isId) { $idCols += $cn }
    }
    if ($insertCols.Count -eq 0) { $dataLogLines += "SKIP $full  no insertable cols"; continue }

    $colList = ($insertCols | ForEach-Object { "[$_]" }) -join ', '
    # Use SELECT * to avoid Chinese column name encoding issues
    $selectSql = "SELECT TOP 1000 * FROM [$schema].[$name]"

    try {
        $cmd = $rc.CreateCommand()
        $cmd.CommandTimeout = 180
        $cmd.CommandText = $selectSql
        $rdr = $cmd.ExecuteReader()

        # Build a mapping of reader column ordinal -> insert column name (skip computed)
        $rdrCols = @()
        for ($c = 0; $c -lt $rdr.FieldCount; $c++) {
            $rdrName = $rdr.GetName($c)
            if ($colMap.ContainsKey($rdrName) -and -not $colMap[$rdrName].IsComputed) {
                $rdrCols += $rdrName
            }
        }

        $rows = 0
        $insSb = New-Object System.Text.StringBuilder
        if ($idCols.Count -gt 0) { [void]$insSb.AppendLine("SET IDENTITY_INSERT [$schema].[$name] ON;") }
        [void]$insSb.AppendLine("DELETE FROM [$schema].[$name];")
        while ($rdr.Read()) {
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
            $rows++
        }
        $rdr.Close()
        if ($idCols.Count -gt 0) { [void]$insSb.AppendLine("SET IDENTITY_INSERT [$schema].[$name] OFF;") }

        if ($rows -gt 0) {
            [void]$dataSb.AppendLine("-- $full  ($rows rows)")
            [void]$dataSb.AppendLine($insSb.ToString())
            [void]$dataSb.AppendLine("GO")
            [void]$dataSb.AppendLine("")
            $dataLogLines += "OK   $full  rows=$rows"
        } else {
            $dataLogLines += "EMPTY $full"
        }
    } catch {
        $msg = $_.Exception.Message
        $stk = $_.Exception.StackTrace
        $dataLogLines += "ERR  $full  $msg"
        $dataLogLines += "     StackTrace: $stk"
        try { if ($rdr -and -not $rdr.IsClosed) { $rdr.Close() } } catch {}
    }
}

# re-enable constraints
[void]$dataSb.AppendLine("EXEC sp_msforeachtable 'ALTER TABLE ? WITH CHECK CHECK CONSTRAINT ALL';")
[void]$dataSb.AppendLine("GO")
[void]$dataSb.AppendLine("")

[System.IO.File]::WriteAllText($dataFile, $dataSb.ToString(), [System.Text.Encoding]::UTF8)
[System.IO.File]::WriteAllText($dataLog, ($dataLogLines -join "`r`n"), [System.Text.Encoding]::UTF8)
Log "Data file: $dataFile  size=$([math]::Round((Get-Item $dataFile).Length / 1MB, 2)) MB"
Log "Data log:  $dataLog  count=$($dataLogLines.Count)"
$rc.Close()
Log "Done."
