#requires -Version 5.0
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'

$dir = 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\failed_tables'
$localCs = "Server=127.0.0.1,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"
$remoteCs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"

$tables = @(
    'dbo.tBas_Emp',
    'dbo.tBas_EmpApply',
    'dbo.tmp_tbas_Emp',
    'dbo.tOA_LineMan',
    'dbo.tSys_MD'
)

$rc = New-Object System.Data.SqlClient.SqlConnection $remoteCs
$rc.Open()
$lc = New-Object System.Data.SqlClient.SqlConnection $localCs
$lc.Open()

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

foreach ($full in $tables) {
    Write-Host "=== $full ==="
    $parts = $full -split '\.'
    $schema = $parts[0]
    $name = $parts[1]
    $safeName = $full -replace '\.','_'
    $csvFile = Join-Path $dir "$safeName.csv"

    # Get column metadata from REMOTE (use system_type_id to handle user-defined types)
    $rawC = Run-Query $rc "SELECT c.column_id, c.name, st.name AS sys_type, c.is_computed, c.is_identity, c.max_length FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id INNER JOIN sys.types st ON c.system_type_id = st.user_type_id WHERE c.object_id = OBJECT_ID('$full') ORDER BY c.column_id"
    if ($null -eq $rawC -or $rawC.Rows.Count -eq 0) {
        Write-Host "  No columns found"
        continue
    }

    # Get LOCAL column metadata for schema comparison
    $rawLocal = Run-Query $lc "SELECT c.column_id, c.name, t.name, c.is_computed, c.is_identity, c.max_length, c.is_nullable FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id WHERE c.object_id = OBJECT_ID('$full') ORDER BY c.column_id"

    $localColNames = @{}
    $localColNullable = @{}
    for ($i = 0; $i -lt $rawLocal.Rows.Count; $i++) {
        $items = $rawLocal.Rows[$i].ItemArray
        $cn = [string]$items[1]
        $localColNames[$cn] = $true
        $localColNullable[$cn] = [bool]$items[6]
    }

    $colNames = @()
    $colTypes = @()
    $colIsId = @()
    $colIsComp = @()
    for ($i = 0; $i -lt $rawC.Rows.Count; $i++) {
        $items = $rawC.Rows[$i].ItemArray
        $cn = [string]$items[1]
        if (-not $localColNames.ContainsKey($cn)) {
            Write-Host "  RemoteOnly col (not in local): $cn - skipping"
            continue
        }
        $colNames += $cn
        $colTypes += [string]$items[2]
        $colIsId += [bool]$items[4]
        $colIsComp += [bool]$items[3]
    }
    Write-Host ("  Types sample: {0}" -f (($colTypes | Select-Object -First 5) -join ','))
    $insertable = @()
    for ($i = 0; $i -lt $colNames.Count; $i++) {
        if (-not $colIsComp[$i]) { $insertable += $colNames[$i] }
    }

    # Read CSV
    $lines = Get-Content $csvFile -Encoding UTF8
    if ($lines.Count -lt 2) {
        Write-Host "  No data rows"
        continue
    }
    $headerCols = $lines[0] -split "`t"
    $headerIdx = @{}
    for ($i = 0; $i -lt $headerCols.Count; $i++) {
        $headerIdx[$headerCols[$i]] = $i
    }
    # Build reader col idx -> insert col name (only insertable cols)
    $rdrToInsertCol = @()
    foreach ($cn in $insertable) {
        if ($headerIdx.ContainsKey($cn)) {
            $rdrToInsertCol += $headerIdx[$cn]
        } else {
            Write-Host "  WARN: missing column $cn in CSV"
            $rdrToInsertCol += -1
        }
    }

    # Build destination DataTable
    $dt = New-Object System.Data.DataTable
    foreach ($cn in $insertable) {
        $idx = [array]::IndexOf($colNames, $cn)
        $t = $colTypes[$idx]
        $netType = switch ($t) {
            'int'            { [int] }
            'bigint'         { [long] }
            'smallint'       { [int16] }
            'tinyint'        { [byte] }
            'bit'            { [bool] }
            'decimal'        { [decimal] }
            'numeric'        { [decimal] }
            'money'          { [decimal] }
            'smallmoney'     { [decimal] }
            'float'          { [double] }
            'real'           { [single] }
            'datetime'       { [datetime] }
            'datetime2'      { [datetime] }
            'smalldatetime'  { [datetime] }
            'date'           { [datetime] }
            'time'           { [timespan] }
            'datetimeoffset' { [datetime] }
            'uniqueidentifier' { [guid] }
            'binary'         { [byte[]] }
            'varbinary'      { [byte[]] }
            default          { [string] }
        }
        $col = New-Object System.Data.DataColumn $cn, $netType
        $col.AllowDBNull = [bool]$localColNullable[$cn]
        $dt.Columns.Add($col)
    }
    Write-Host ("  Cols: {0}" -f (($dt.Columns | ForEach-Object { "{0}:{1}" -f $_.ColumnName, $_.DataType.Name }) -join ', '))

    # Parse data rows
    for ($li = 1; $li -lt $lines.Count; $li++) {
        $line = $lines[$li]
        if ([string]::IsNullOrEmpty($line)) { continue }
        $fields = $line -split "`t"
        $dr = $dt.NewRow()
        for ($i = 0; $i -lt $rdrToInsertCol.Count; $i++) {
            $fi = $rdrToInsertCol[$i]
            if ($fi -lt 0) { $dr[$i] = [DBNull]::Value; continue }
            $val = $fields[$fi]
            $colName = $insertable[$i]
            $colIdx = [array]::IndexOf($colNames, $colName)
            $colType = $colTypes[$colIdx]
            if ($colType -eq 'uniqueidentifier') {
                if ([string]::IsNullOrEmpty($val)) {
                    if (-not $localColNullable[$colName]) {
                        $dr[$i] = [guid]'00000000-0000-0000-0000-000000000000'
                    } else {
                        $dr[$i] = [DBNull]::Value
                    }
                } else {
                    $dr[$i] = [guid]$val
                }
            } elseif ($colType -eq 'bit') {
                if ([string]::IsNullOrEmpty($val)) {
                    if (-not $localColNullable[$colName]) {
                        $dr[$i] = $false
                    } else {
                        $dr[$i] = [DBNull]::Value
                    }
                } else {
                    $dr[$i] = [bool]([int]$val)
                }
            } elseif ($colType -in 'int','bigint','smallint','tinyint') {
                if ([string]::IsNullOrEmpty($val)) {
                    if (-not $localColNullable[$colName]) {
                        $dr[$i] = 0
                    } else {
                        $dr[$i] = [DBNull]::Value
                    }
                } else {
                    $dr[$i] = [int]$val
                }
            } elseif ($colType -in 'decimal','numeric','money','smallmoney') {
                if ([string]::IsNullOrEmpty($val)) {
                    if (-not $localColNullable[$colName]) {
                        $dr[$i] = 0
                    } else {
                        $dr[$i] = [DBNull]::Value
                    }
                } else {
                    $dr[$i] = [decimal]$val
                }
            } elseif ($colType -in 'datetime','datetime2','smalldatetime','date') {
                if ([string]::IsNullOrEmpty($val)) {
                    if (-not $localColNullable[$colName]) {
                        $dr[$i] = [datetime]'1900-01-01'
                    } else {
                        $dr[$i] = [DBNull]::Value
                    }
                } else {
                    $dr[$i] = [datetime]::Parse($val)
                }
            } elseif ($colType -in 'binary','varbinary') {
                if ([string]::IsNullOrEmpty($val)) {
                    if (-not $localColNullable[$colName]) {
                        $dr[$i] = [byte[]]@()
                    } else {
                        $dr[$i] = [DBNull]::Value
                    }
                } else {
                    $b = for ($k=0; $k -lt $val.Length; $k += 2) { [Convert]::ToByte($val.Substring($k,2),16) }
                    $dr[$i] = [byte[]]$b
                }
            } elseif ([string]::IsNullOrEmpty($val)) {
                if (-not $localColNullable[$colName]) {
                    $dr[$i] = ''
                } else {
                    $dr[$i] = [DBNull]::Value
                }
            } else {
                $dr[$i] = $val
            }
        }
        $dt.Rows.Add($dr)
    }

    # DELETE local, then BULK COPY
    $delCmd = $lc.CreateCommand()
    $delCmd.CommandText = "DELETE FROM [$schema].[$name]"
    $delCmd.CommandTimeout = 300
    Write-Host "  Deleting local rows..."
    $delN = $delCmd.ExecuteNonQuery()
    Write-Host "  Deleted $delN rows"

    # BulkCopy
    $bulk = New-Object System.Data.SqlClient.SqlBulkCopy $lc
    $bulk.DestinationTableName = "[$schema].[$name]"
    $bulk.BatchSize = 100
    $bulk.BulkCopyTimeout = 600
    foreach ($cn in $insertable) {
        $m = New-Object System.Data.SqlClient.SqlBulkCopyColumnMapping $cn, $cn
        $bulk.ColumnMappings.Add($m)
    }
    Write-Host "  BulkCopying $($dt.Rows.Count) rows..."
    $bulk.WriteToServer($dt)
    $bulk.Close()
    Write-Host "  OK"
}
$lc.Close()
$rc.Close()
Write-Host "Done"
