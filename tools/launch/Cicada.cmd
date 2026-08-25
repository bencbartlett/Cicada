@echo off
setlocal
title Cicada
rem Cicada dev launcher (Windows; docs/17 wave 4 L3). Double-click: this
rem window is the terminal. It finds Python 3 and hands over to launch.py,
rem which builds cicada in release with the app embedded when it is missing
rem or stale, puts the kernel's run-time libraries beside it, and runs
rem `cicada app` (arguments given here go to it). Every failure is printed
rem and this window stays open on one.
set "HERE=%~dp0"
set "PY="
python -c "import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)" >nul 2>nul && set "PY=python"
if not defined PY py -3 -c "import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)" >nul 2>nul && set "PY=py -3"
if not defined PY (
  echo error: Python 3.9 or newer is required and neither `python` nor `py -3` is one.
  echo        Install it from https://www.python.org/downloads/ ^(tick "Add python.exe to PATH"^)
  echo        or with `winget install Python.Python.3.12`, then run this again.
  pause
  exit /b 1
)
%PY% "%HERE%launch.py" %*
set "CODE=%ERRORLEVEL%"
if not "%CODE%"=="0" (
  echo.
  echo Cicada's launcher stopped with exit code %CODE% -- see the messages above.
  pause
)
exit /b %CODE%
