@echo off
REM ============================================================
REM ERP Backend one-key restart (Windows)
REM Pass-through: release / debug / -NoBuild / -NoStart / -NoTail / -StopOnly
REM ============================================================
chcp 65001 > nul
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0restart_backend.ps1" %*