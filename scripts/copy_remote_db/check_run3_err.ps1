$log = [System.IO.File]::ReadAllText('C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\02_data_run3.log', [System.Text.Encoding]::GetEncoding(936))
$lines = $log -split "`n"
$lines | Select-Object -Last 60
