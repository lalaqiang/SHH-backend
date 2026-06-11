#requires -Version 5.0
# Test which columns need special handling
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'

$RemoteServer = 'shenhuahui.f3322.org,1433'
$RemoteDb     = 'TestERP'
$RemoteUser   = 'sa'
$RemotePwd    = 'sa123456'

Add-Type -AssemblyName 'Microsoft.SqlServer.SMO'

$remoteSrv = New-Object Microsoft.SqlServer.Management.Smo.Server($RemoteServer)
$remoteSrv.ConnectionContext.LoginSecure = $false
$remoteSrv.ConnectionContext.Login = $RemoteUser
$remoteSrv.ConnectionContext.Password = $RemotePwd
$remoteSrv.ConnectionContext.ConnectTimeout = 60
$remoteSrv.SetDefaultInitFields([Microsoft.SqlServer.Management.Smo.Table], $true)
$remoteDb = $remoteSrv.Databases[$RemoteDb]

$testTables = @('dbo.tBas_Dept', 'dbo.tBas_Goods', 'dbo.tSal_Inv', 'dbo.tStk_IO', 'dbo.tStk_Stock', 'dbo.tSys_OperHis', 'dbo.tArd_Log', 'dbo.tStk_IODetail', 'dbo.goods', 'dbo.brand')
foreach ($tn in $testTables) {
    $parts = $tn.Split('.')
    $t = $remoteDb.Tables[$parts[1], $parts[0]]
    if ($null -eq $t) { Write-Host "$tn NOT FOUND" -ForegroundColor Red; continue }
    Write-Host "================================================" -ForegroundColor Yellow
    Write-Host ("Table: {0}  Rows: {1}  Cols: {2}" -f $tn, $t.RowCount, $t.Columns.Count) -ForegroundColor Yellow

    $idCols = @(); $compCols = @(); $blobCols = @(); $fkCount = 0; $ckCount = 0
    foreach ($c in $t.Columns) {
        if ($c.Identity) { $idCols += $c.Name }
        if ($c.Computed) { $compCols += $c.Name }
        $sd = $c.DataType.SqlDataType.ToString()
        if ($sd -in 'VarBinary','Binary','Image','Timestamp','RowVersion','VarBinaryMax','UDT') { $blobCols += "$($c.Name)($sd)" }
    }
    $fkCount = $t.ForeignKeys.Count
    $ckCount = $t.Checks.Count
    if ($idCols.Count -gt 0)   { Write-Host ("  Identity: {0}" -f ($idCols -join ', ')) -ForegroundColor Magenta }
    if ($compCols.Count -gt 0) { Write-Host ("  Computed: {0}" -f ($compCols -join ', ')) -ForegroundColor Magenta }
    if ($blobCols.Count -gt 0) { Write-Host ("  Binary: {0}" -f ($blobCols -join ', ')) -ForegroundColor Magenta }
    if ($fkCount -gt 0)         { Write-Host ("  FKs: {0}" -f $fkCount) -ForegroundColor Magenta }
    if ($ckCount -gt 0)         { Write-Host ("  CheckConstraints: {0}" -f $ckCount) -ForegroundColor Magenta }
}
