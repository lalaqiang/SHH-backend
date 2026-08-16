-- Expand tSys_TableColumnConfig.ConfigData from nvarchar(4000) to nvarchar(max)
-- Run this once to avoid "列配置 JSON 长度接近 nvarchar(4000) 上限" 报错
-- Usage: sqlcmd -S 'DESKTOP-QKTHTQP\SQLEXPRESS' -d TestERP -i expand_column_config_max.sql

ALTER TABLE tSys_TableColumnConfig ALTER COLUMN ConfigData nvarchar(max) NULL;
PRINT 'tSys_TableColumnConfig.ConfigData -> nvarchar(max) OK';
