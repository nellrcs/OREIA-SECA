@echo off
setlocal
echo ========================================================
echo  OREIA-SECA: Reset de Containers e Volumes Temporarios
echo ========================================================
echo.

echo [1/3] Parando containers e removendo volumes docker...
docker compose down -v --remove-orphans

echo [2/3] Limpando dados persistidos em ./volumes...
if exist volumes\postgres rmdir /s /q volumes\postgres
if exist volumes\forgejo rmdir /s /q volumes\forgejo
if exist volumes\px0 rmdir /s /q volumes\px0
if exist volumes\oreia-seca\data rmdir /s /q volumes\oreia-seca\data
if exist volumes\oreia-seca\workspace rmdir /s /q volumes\oreia-seca\workspace
if exist volumes\oreia-seca\tmp rmdir /s /q volumes\oreia-seca\tmp

echo [3/3] Recriando diretorios base...
mkdir volumes\oreia-seca\data 2>nul
mkdir volumes\oreia-seca\workspace 2>nul
mkdir volumes\oreia-seca\tmp 2>nul

echo.
echo ========================================================
echo  Reset concluido com sucesso!
echo  Suas configuracoes (.env e oreia-seca.toml) foram mantidas.
echo  Para iniciar o sistema:
echo    .\oreia bootstrap
echo    .\oreia up
echo ========================================================
