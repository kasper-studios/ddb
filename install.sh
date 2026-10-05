#!/usr/bin/env bash
# ==============================================================================
#               DDB (Dirom Device Bridge) — Universal Installer
#                   Designed by Kasperenok & Dirom Architecture
# ==============================================================================
set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
PURPLE='\033[0;35m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${PURPLE}"
cat << "EOF"
         👑
      .---^---.
     /   DDB   \
    /  (B B D)  \
   |  =========  |   DIROM DEVICE BRIDGE
   |  *  *  *  * |   Universal Native Host Agent
    \___________/    Kasper Studios Citadel Architecture
EOF
echo -e "${NC}"

echo -e "${CYAN}==> [1/5] Проверка окружения...${NC}"
if ! command -v node &> /dev/null; then
    echo -e "${RED}Ошибка: Node.js не найден. Требуется Node.js 20+ для запуска DDB.${NC}"
    exit 1
fi

NODE_VER=$(node -v)
echo -e "${GREEN}✓ Node.js обнаружен: ${NODE_VER}${NC}"

INSTALL_DIR="${HOME}/.local/share/ddb"
CONFIG_DIR="${HOME}/.config/ddb"

echo -e "${CYAN}==> [2/5] Подготовка директорий...${NC}"
mkdir -p "${INSTALL_DIR}" "${CONFIG_DIR}"

# Генерация секретного токена если не существует
TOKEN_FILE="${CONFIG_DIR}/token"
if [ ! -f "${TOKEN_FILE}" ]; then
    DDB_TOKEN=$(openssl rand -hex 24 2>/dev/null || date +%s%N | sha256sum | head -c 48)
    echo "${DDB_TOKEN}" > "${TOKEN_FILE}"
    chmod 600 "${TOKEN_FILE}"
    echo -e "${GREEN}✓ Сгенерирован новый защищённый токен доступа.${NC}"
else
    DDB_TOKEN=$(cat "${TOKEN_FILE}")
    echo -e "${YELLOW}! Использован существующий токен доступа.${NC}"
fi

echo -e "${CYAN}==> [3/5] Детекция системных возможностей (Capabilities)...${NC}"
HAS_NIRI=false
HAS_WAYLAND=false
HAS_PIPEWIRE=false
HAS_CLOUDFLARED=false

command -v niri &> /dev/null && HAS_NIRI=true
[ -n "$WAYLAND_DISPLAY" ] && HAS_WAYLAND=true
command -v pw-cat &> /dev/null && HAS_PIPEWIRE=true
command -v cloudflared &> /dev/null && HAS_CLOUDFLARED=true

echo -e "  * Wayland: ${HAS_WAYLAND}"
echo -e "  * Niri WM: ${HAS_NIRI}"
echo -e "  * PipeWire: ${HAS_PIPEWIRE}"
echo -e "  * Cloudflared: ${HAS_CLOUDFLARED}"

echo -e "${GREEN}✓ DDB готов к установке служб systemd user.${NC}"
echo -e "${CYAN}================================================================${NC}"
echo -e "${GREEN}Реквизиты моста:${NC}"
echo -e "  Main API Port:      ${YELLOW}:8780${NC}"
echo -e "  Emergency Shell:    ${RED}:8790${NC}"
echo -e "  Auth Token:         ${PURPLE}${DDB_TOKEN}${NC}"
echo -e "${CYAN}================================================================${NC}"
