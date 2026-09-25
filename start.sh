#!/usr/bin/env bash
set -e

echo "========================================================"
echo "              IRCord Docker Launcher"
echo "========================================================"
echo ""
echo "Select the version you want to run:"
echo ""
echo " [1] Ollama ROCm + GPU (Recommended for Minisforum N5 Pro)"
echo "     - Starts Ollama with AMD Radeon 890M GPU support (/dev/kfd, /dev/dri)"
echo "     - Automatically pulls configured AI model (e.g. qwen2.5:7b)"
echo "     - Starts IRCord daemon"
echo ""
echo " [2] FreeToken ROCm"
echo "     - Starts FlashML FreeToken AI engine"
echo "     - Starts IRCord daemon"
echo ""
echo " [3] Standalone (No local AI container)"
echo "     - Starts IRCord daemon only"
echo "     - Connects to host Ollama (host.docker.internal:11434) or cloud API"
echo ""
echo " [4] Stop all IRCord containers"
echo " [5] View live logs (docker compose logs -f)"
echo " [0] Exit"
echo ""
echo "========================================================"
read -rp "Enter your choice [1-5]: " choice

case "$choice" in
  1)
    echo "[INFO] Launching with docker-compose.ollama.yml..."
    docker compose -f docker-compose.ollama.yml up -d --build
    ;;
  2)
    echo "[INFO] Launching with docker-compose.freetoken.yml..."
    docker compose -f docker-compose.freetoken.yml up -d --build
    ;;
  3)
    echo "[INFO] Launching with docker-compose.standalone.yml..."
    docker compose -f docker-compose.standalone.yml up -d --build
    ;;
  4)
    echo "[INFO] Stopping all containers..."
    docker compose -f docker-compose.ollama.yml down 2>/dev/null || true
    docker compose -f docker-compose.freetoken.yml down 2>/dev/null || true
    docker compose -f docker-compose.standalone.yml down 2>/dev/null || true
    ;;
  5)
    echo "[INFO] Opening logs..."
    docker compose logs -f
    ;;
  0)
    exit 0
    ;;
  *)
    echo "Invalid choice."
    exit 1
    ;;
esac
