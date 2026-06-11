#requires -Version 5.0
# For each table in to_process.csv, export first N rows from remote, then bulk load to local
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Continue'

$dir = 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\reload_tables'
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }

$localCs = "Server=127.0.0.1,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"
$remoteCs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"

function Run-Query($conn, [string]$sql) {
    $cmd = $conn.CreateCommand()
    $cmd.CommandTimeout = 600
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

$tables = Import-Csv (Join-Path $dir 'to_process.csv')
Write-Host ("Total tables to process: {0}" -f $tables.Count)

$success = 0
$failed = 0
$skipped = 0
$idx = 0
foreach ($t in $tables) {
    $idx++
    $full = $t.Full
    $parts = $full -split '\.'
    $schema = $parts[0]
    $name = $parts[1]
    $expected = [int]$t.Expected
    Write-Progress -Activity "Reloading tables" -Status "$full ($idx/$($tables.Count))" -PercentComplete ([math]::Round($idx * 100 / $tables.Count, 1))
    Write-Host "[$idx/$($tables.Count)] $full  (expected=$expected)"

    if ($expected -eq 0) {
        # Just DELETE all local rows
        try {
            $delCmd = $lc.CreateCommand()
            $delCmd.CommandText = "DELETE FROM [$schema].[$name]"
            $delCmd.CommandTimeout = 600
            $n = $delCmd.ExecuteNonQuery()
            Write-Host "  DELETED $n rows (expected 0)"
            $success++
        } catch {
            Write-Host "  ERROR deleting: $($_.Exception.Message)"
            $failed++
        }
        continue
    }

    # Get column metadata from remote
    $rawC = Run-Query $rc "SELECT c.column_id, c.name, st.name AS sys_type, c.is_computed, c.is_identity FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id INNER JOIN sys.types st ON c.system_type_id = st.user_type_id WHERE c.object_id = OBJECT_ID('$full') ORDER BY c.column_id"
    if ($null -eq $rawC -or $rawC.Rows.Count -eq 0) {
        Write-Host "  No columns"
        $skipped++
        continue
    }

    # Get local column metadata
    $rawLocal = Run-Query $lc "SELECT c.name, c.is_nullable FROM sys.columns c WHERE c.object_id = OBJECT_ID('$full')"
    $localColNames = @{}
    $localColNullable = @{}
    foreach ($lr in $rawLocal.Rows) {
        $cn = [string]$lr[0]
        $localColNames[$cn] = $true
        $localColNullable[$cn] = [bool]$lr[1]
    }

    $colNames = @()
    $colTypes = @()
    $colIsId = @()
    $colIsComp = @()
    for ($i = 0; $i -lt $rawC.Rows.Count; $i++) {
        $items = $rawC.Rows[$i].ItemArray
        $cn = [string]$items[1]
        if (-not $localColNames.ContainsKey($cn)) { continue }
        $colNames += $cn
        $colTypes += [string]$items[2]
        $colIsId += [bool]$items[4]
        $colIsComp += [bool]$items[3]
    }

    $insertable = @()
    for ($i = 0; $i -lt $colNames.Count; $i++) {
        if (-not $colIsComp[$i]) { $insertable += $colNames[$i] }
    }

    if ($insertable.Count -eq 0) {
        Write-Host "  No insertable cols"
        $skipped++
        continue
    }

    # Read all data from remote
    try {
        $cmd = $rc.CreateCommand()
        $cmd.CommandTimeout = 600
        $cmd.CommandText = "SELECT TOP $expected * FROM [$schema].[$name]"
        $rdr = $cmd.ExecuteReader()

        # Build a mapping of reader column ordinal -> insert column name
        $rdrCols = @()
        for ($c = 0; $c -lt $rdr.FieldCount; $c++) {
            $rdrName = $rdr.GetName($c)
            if ($colNames -contains $rdrName -and -not $colIsComp[[array]::IndexOf($colNames, $rdrName)]) {
                $rdrCols += $rdrName
            }
        }

        # Build DataTable
        $dt = New-Object System.Data.DataTable
        foreach ($cn in $insertable) {
            $idx2 = [array]::IndexOf($colNames, $cn)
            $t2 = $colTypes[$idx2]
            $netType = switch ($t2) {
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
                'image'          { [byte[]] }
                'timestamp'      { [byte[]] }
                default          { [string] }
            }
            $col = New-Object System.Data.DataColumn $cn, $netType
            $col.AllowDBNull = [bool]$localColNullable[$cn]
            $dt.Columns.Add($col)
        }

        $rowsRead = 0
        while ($rdr.Read()) {
            $dr = $dt.NewRow()
            for ($c = 0; $c -lt $rdr.FieldCount; $c++) {
                $rdrName = $rdr.GetName($c)
                $i2 = [array]::IndexOf($insertable, $rdrName)
                if ($i2 -lt 0) { continue }
                if ($rdr.IsDBNull($c)) {
                    if (-not $localColNullable[$rdrName]) {
                        # substitute default
                        $ct = $colTypes[[array]::IndexOf($colNames, $rdrName)]
                        switch ($ct) {
                            'uniqueidentifier' { $dr[$i2] = [guid]'00000000-0000-0000-0000-000000000000' }
                            'bit'              { $dr[$i2] = $false }
                            'int'              { $dr[$i2] = 0 }
                            'bigint'           { $dr[$i2] = 0L }
                            'smallint'         { $dr[$i2] = [int16]0 }
                            'tinyint'          { $dr[$i2] = [byte]0 }
                            { $_ -in 'decimal','numeric','money','smallmoney' } { $dr[$i2] = 0 }
                            { $_ -in 'datetime','datetime2','smalldatetime','date' } { $dr[$i2] = [datetime]'1900-01-01' }
                            { $_ -in 'binary','varbinary','image','timestamp' } { $dr[$i2] = [byte[]]@() }
                            default            { $dr[$i2] = '' }
                        }
                    } else {
                        $dr[$i2] = [DBNull]::Value
                    }
                } else {
                    $v = $rdr.GetValue($c)
                    $ct = $colTypes[[array]::IndexOf($colNames, $rdrName)]
                    if ($ct -eq 'uniqueidentifier' -and $v -isnot [guid]) {
                        $dr[$i2] = [guid]$v
                    } elseif ($ct -eq 'bit') {
                        $dr[$i2] = [bool]$v
                    } else {
                        $dr[$i2] = $v
                    }
                }
            }
            $dt.Rows.Add($dr)
            $rowsRead++
        }
        $rdr.Close()

        # DELETE local
        $delCmd = $lc.CreateCommand()
        $delCmd.CommandText = "DELETE FROM [$schema].[$name]"
        $delCmd.CommandTimeout = 600
        $delCmd.ExecuteNonQuery() | Out-Null

        # BulkCopy
        if ($dt.Rows.Count -gt 0) {
            $bulk = New-Object System.Data.SqlClient.SqlBulkCopy $lc
            $bulk.DestinationTableName = "[$schema].[$name]"
            $bulk.BatchSize = 100
            $bulk.BulkCopyTimeout = 600
            foreach ($cn in $insertable) {
                $m = New-Object System.Data.SqlClient.SqlBulkCopyColumnMapping $cn, $cn
                $bulk.ColumnMappings.Add($m)
            }
            $bulk.WriteToServer($dt) 2>&1 | Out-Null
            $bulk.Close()
        }
        Write-Host "  OK: read=$rowsRead"
        $success++
    } catch {
        Write-Host "  ERROR: $($_.Exception.Message)"
        $failed++
    }
}
$lc.Close()
$rc.Close()
Write-Host ""
Write-Host ("Success: {0}, Failed: {1}, Skipped: {2}" -f $success, $failed, $skipped)
