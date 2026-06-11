#requires -Version 5.0
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'

$dir = 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\failed_tables'
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }

$tables = @(
    'dbo.tBas_Emp',
    'dbo.tBas_EmpApply',
    'dbo.tmp_tbas_Emp',
    'dbo.tOA_LineMan',
    'dbo.tSys_MD'
)

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

# Test approach: For each table, extract data to CSV file using SqlClient, then BULK INSERT
$localCs = "Server=127.0.0.1,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"
$remoteCs = "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"

$rc = New-Object System.Data.SqlClient.SqlConnection $remoteCs
$rc.Open()

foreach ($full in $tables) {
    Write-Host "=== $full ==="
    $parts = $full -split '\.'
    $schema = $parts[0]
    $name = $parts[1]
    $safeName = $full -replace '\.','_'
    $csvFile = Join-Path $dir "$safeName.csv"
    $fmtFile = Join-Path $dir "$safeName.fmt"

    # Get column metadata
    $rawC = Run-Query $rc "SELECT c.column_id, c.name, t.name, c.is_computed, c.is_identity, c.is_nullable, c.max_length FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id WHERE c.object_id = OBJECT_ID('$full') ORDER BY c.column_id"
    if ($null -eq $rawC -or $rawC.Rows.Count -eq 0) {
        Write-Host "  No columns found"
        continue
    }

    # Build column list and meta
    $colNames = @()
    $colTypes = @()
    $colIsId = @()
    $colIsComp = @()
    $colLen = @()
    for ($i = 0; $i -lt $rawC.Rows.Count; $i++) {
        $items = $rawC.Rows[$i].ItemArray
        $colNames += [string]$items[1]
        $colTypes += [string]$items[2]
        $colIsId += [bool]$items[4]
        $colIsComp += [bool]$items[3]
        $colLen += [int]$items[6]
    }

    # Read all data
    $cmd = $rc.CreateCommand()
    $cmd.CommandTimeout = 300
    $cmd.CommandText = "SELECT TOP 1000 * FROM [$schema].[$name]"
    $rdr = $cmd.ExecuteReader()
    $rows = 0
    $sb = New-Object System.Text.StringBuilder
    # Use tab separator to avoid issues with commas in values
    $sep = "`t"
    $colLine = ($colNames | Where-Object { -not $colIsComp[[array]::IndexOf($colNames, $_)] }) -join $sep
    [void]$sb.AppendLine($colLine)
    while ($rdr.Read()) {
        $vals = @()
        for ($c = 0; $c -lt $rdr.FieldCount; $c++) {
            $rdrName = $rdr.GetName($c)
            $idx = [array]::IndexOf($colNames, $rdrName)
            if ($idx -lt 0 -or $colIsComp[$idx]) { continue }
            if ($rdr.IsDBNull($c)) {
                $vals += ''  # empty for null
            } else {
                $v = $rdr.GetValue($c)
                if ($v -is [string]) {
                    $s = $v -replace "`t","    " -replace "`r`n",' ' -replace "`n",' '
                    $s = $s -replace "`r",' '
                    $vals += $s
                } elseif ($v -is [DateTime]) {
                    $vals += $v.ToString('yyyy-MM-dd HH:mm:ss.fff')
                } elseif ($v -is [bool]) {
                    $b = if ($v) {'1'} else {'0'}
                    $vals += $b
                } elseif ($v -is [byte[]]) {
                    $bytes = [byte[]]$v
                    if ($bytes.Length -eq 0) { $vals += '' } else { $vals += ('0x' + [BitConverter]::ToString($bytes).Replace('-','')) }
                } else {
                    $vals += [string]$v
                }
            }
        }
        [void]$sb.AppendLine(($vals -join $sep))
        $rows++
    }
    $rdr.Close()
    [System.IO.File]::WriteAllText($csvFile, $sb.ToString(), [System.Text.Encoding]::UTF8)
    Write-Host "  Wrote $rows rows to $csvFile"
}
$rc.Close()
Write-Host "Done"
