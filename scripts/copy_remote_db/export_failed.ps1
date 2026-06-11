$ErrorActionPreference = 'Stop'
$tables = @('dbo.tBas_Emp','dbo.tBas_EmpApply','dbo.tmp_tbas_Emp','dbo.tOA_LineMan','dbo.tSys_MD')
$dir = 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\failed_tables'
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }

foreach ($t in $tables) {
    $safe = ($t -replace '\.','_')
    $file = Join-Path $dir "$safe.dat"
    Write-Host "Exporting $t to $file"
    # bcp uses -c for char format with tabs/newlines as separators, with -C 936 for GBK
    $arg = "shenhuahui.f3322.org,1433 -U sa -P sa123456 -n -S `"" -q`" -d TestERP -c -t | -r |\n`" --out=$file --query=`"SELECT TOP 1000 * FROM $t ORDER BY (SELECT NULL)`" 2>&1"
    Write-Host "  Trying native bcp..."
    try {
        $proc = Start-Process -FilePath "bcp" -ArgumentList @($t,'out',"`"$file`"","-S","shenhuahui.f3322.org,1433","-U","sa","-P","sa123456","-d","TestERP","-c","-C","936","-t","|","-r","\n","-a","65535") -NoNewWindow -Wait -PassThru -RedirectStandardOutput "stdout.txt" -RedirectStandardError "stderr.txt"
        Write-Host "  ExitCode: $($proc.ExitCode)"
        if ($proc.ExitCode -ne 0) {
            Write-Host "  STDERR:"
            Get-Content stderr.txt
        }
    } catch {
        Write-Host "  Error: $_"
    }
}
Write-Host "Done"
