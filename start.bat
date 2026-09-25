@echo off
chcp 65001 > nul
cls
echo ========================================================
echo               IRCord Docker Launcher
echo ========================================================
echo.
echo Select the version you want to run:
echo.
echo  [1] Ollama ROCm + GPU (Recommended for Minisforum N5 Pro)
echo      - Starts Ollama with AMD Radeon 890M GPU support
echo      - Automatically pulls configured AI model (e.g. qwen2.5:7b)
echo      - Starts IRCord daemon
echo.
echo  [2] FreeToken ROCm
echo      - Starts FlashML FreeToken AI engine
echo      - Starts IRCord daemon
echo.
echo  [3] Standalone (No local AI container)
echo      - Starts IRCord daemon only
echo      - Connects to host Ollama or remote API
echo.
echo  [4] Stop all IRCord containers
echo  [5] View live logs (docker compose logs -f)
echo  [0] Exit
echo.
echo ========================================================
set /p choice="Enter your choice [1-5]: "

if "%choice%"=="1" (
    echo.
    echo [INFO] Launching with docker-compose.ollama.yml...
    docker compose -f docker-compose.ollama.yml up -d --build
    goto end
)
if "%choice%"=="2" (
    echo.
    echo [INFO] Launching with docker-compose.freetoken.yml...
    docker compose -f docker-compose.freetoken.yml up -d --build
    goto end
)
if "%choice%"=="3" (
    echo.
    echo [INFO] Launching with docker-compose.standalone.yml...
    docker compose -f docker-compose.standalone.yml up -d --build
    goto end
)
if "%choice%"=="4" (
    echo.
    echo [INFO] Stopping all containers...
    docker compose -f docker-compose.ollama.yml down
    docker compose -f docker-compose.freetoken.yml down
    docker compose -f docker-compose.standalone.yml down
    goto end
)
if "%choice%"=="5" (
    echo.
    echo [INFO] Opening logs...
    docker compose logs -f
    goto end
)
if "%choice%"=="0" (
    exit /b 0
)

echo Invalid choice.
:end
pause
