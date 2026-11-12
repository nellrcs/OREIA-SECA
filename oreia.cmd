@echo off
rem ---------------------------------------------------------------------------
rem OREIA-SECA no Windows, a partir do cmd ou do PowerShell:
rem
rem     oreia.cmd            instala (na primeira vez) e abre o painel
rem     oreia.cmd install    instalacao guiada
rem     oreia.cmd help       todos os comandos
rem
rem Este arquivo so localiza o bash do "Git for Windows" e repassa tudo para
rem o script "oreia" desta mesma pasta - a logica de verdade esta la, igual
rem para Linux, Mac e Windows. Dentro do Git Bash, use direto:  ./oreia
rem ---------------------------------------------------------------------------
setlocal

set "OREIA_BASH="
if exist "%ProgramFiles%\Git\bin\bash.exe" set "OREIA_BASH=%ProgramFiles%\Git\bin\bash.exe"
if not defined OREIA_BASH if exist "%ProgramFiles(x86)%\Git\bin\bash.exe" set "OREIA_BASH=%ProgramFiles(x86)%\Git\bin\bash.exe"
if not defined OREIA_BASH if exist "%LocalAppData%\Programs\Git\bin\bash.exe" set "OREIA_BASH=%LocalAppData%\Programs\Git\bin\bash.exe"

rem Git instalado em outro lugar: git.exe fica em <Git>\cmd\, o bash em <Git>\bin\.
rem (De proposito NAO usa "where bash": ele costuma achar o C:\Windows\System32\bash.exe
rem do WSL, que roda em outro sistema, sem o Docker Desktop deste.)
if not defined OREIA_BASH for /f "delims=" %%G in ('where git 2^>nul') do call :from_git "%%~dpG"

if not defined OREIA_BASH goto :no_bash

"%OREIA_BASH%" "%~dp0oreia" %*
exit /b %ERRORLEVEL%

:from_git
if defined OREIA_BASH exit /b 0
if exist "%~1..\bin\bash.exe" set "OREIA_BASH=%~1..\bin\bash.exe"
exit /b 0

:no_bash
echo.
echo  Nao encontrei o Git for Windows (ele traz o bash que o OREIA-SECA usa).
echo.
echo  Instale em https://git-scm.com/download/win  (as opcoes padrao servem)
echo  e rode de novo:  oreia.cmd install
echo.
echo  Voce tambem precisa do Docker Desktop: https://docs.docker.com/desktop/setup/install/windows-install/
echo.
exit /b 1
