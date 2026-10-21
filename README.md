# ⚡ DDB (Dirom Device Bridge)

<div align="center">

![Rust](https://img.shields.io/badge/Rust-2024_Edition-orange.svg?style=for-the-badge&logo=rust)
![License](https://img.shields.io/badge/License-MIT-blue.svg?style=for-the-badge)
![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20Windows-blueviolet.svg?style=for-the-badge)
![Status](https://img.shields.io/badge/Status-v1.0.0--Production-success.svg?style=for-the-badge)

**Суверенный кроссплатформенный агент устройств Цитадели и мост Computer Use для автономного ИИ (Дирома и Гермеса).**  
Монолитный нативный бинарник без JS/Node.js рантаймов, написанный с нуля на Rust.

[Скачать Релиз (Linux / Windows)](https://github.com/kasper-studios/ddb/releases/tag/v1.0.0) • [Архитектурный Контракт (DevNotes)](DevNotes.md)

</div>

---

## 🌟 Ключевые Возможности

- 🚀 **Нативный Rust Монолит:** Память ~10–15 МБ ОЗУ, размер бинарника от 6 до 8.5 МБ. Никаких Electron, Python или Node.js зависимостей при запуске.
- 🖱️ **Computer Use & Virtual Pointer:**
  - **Linux (Wayland / Niri / X11):** Нативный драйвер мыши ядра через `/dev/uinput` с кинематическим сглаживанием траектории по кубическим кривым Безье (20–30 шагов, human-like ease-out), клики, хоткеи и Niri IPC.
  - **Windows (10 / 11):** Win32 эмуляция мыши и кликов (`user32.dll` `SetCursorPos` + `mouse_event`), ввод текста и сочетаний клавиш через Windows Forms `SendKeys`.
- 📸 **Display Capture (Скриншоты):**
  - **Linux:** Сверхбыстрый захват экрана в память через `grim` с поддержкой Wayland (Niri) без сохранения временных файлов на диск.
  - **Windows:** Нативный захват первичного монитора в память через GDI/WinForms Graphics.
- 🧠 **Hermes Sibling Synapse (A2A Bridge):**
  - Автоматическое обнаружение локального бэкенда `hermes serve` на хосте через `/proc`.
  - Извлечение токена `HERMES_DASHBOARD_SESSION_TOKEN` и порта.
  - Двусторонний обмен задачами с маркером `[Dirom Reqwest]` и поддержка протокола `prompt.submit`.
- 🛡️ **Гранулярная Матрица Прав (Permissions Matrix):**
  - Хост контролирует каждое действие ИИ через `~/.config/ddb/config.json`.
  - Защита на уровне ядра: шелл (`exec`), скриншоты (`screen`), ввод (`display_control`), чтение/запись файлов (`fs.*`), связь с Гермесом (`hermes`).
- 🌐 **Сетевые Режимы и Туннели:**
  - Прямой P2P LAN (локальная сеть планшета и хоста).
  - Kasper Cloud API (Zero-Config WSS к облаку Цитадели).
  - Cloudflare Tunnel & Ngrok.
- 📟 **Интерактивный TUI-конфигуратор (Ratatui):**
  - Живое переключение тумблеров прав прямо в терминале по клавишам `1`–`8` или `Enter`.
  - Быстрая установка службы в `systemd --user` одной кнопкой.

---

## 📦 Быстрый Старт

### 1. Скачивание готового бинарника

Загрузите исполняемый файл для вашей платформы со страницы [Releases v1.0.0](https://github.com/kasper-studios/ddb/releases/tag/v1.0.0):
- **Linux:** `ddb-linux-x64`
- **Windows:** `ddb-windows-x64.exe`

#### Linux:
```bash
chmod +x ddb-linux-x64
./ddb-linux-x64
```

### 2. Сборка из исходников

Требуется Rust 1.85+ (Edition 2024).

```bash
# Клонирование репозитория
git clone https://github.com/kasper-studios/ddb.git
cd ddb

# Сборка публичной версии под Linux
cargo build --release

# Сборка Kasper Edition с полной поддержкой Hermes A2A
cargo build --release --features hermes

# Кросс-компиляция под Windows (через cargo-xwin)
cargo xwin build --release --target x86_64-pc-windows-msvc
```

---

## 🎮 Режимы Работы

### 1. Интерактивный TUI-конфигуратор
Запуск без аргументов открывает графический терминальный дашборд:
```bash
./ddb
```
- Перемещайтесь стрелками `↑` / `↓` или цифрами `1`–`6`.
- В меню **[1] Настройка прав доступа** переключайте разрешения пробелом или Enter — изменения мгновенно сохраняются.
- В шапке генерируется готовая строка для регистрации устройства в Дироме:
  ```python
  device.reg(name='ponosik', url='http://192.168.1.100:8790', key='ddb_...')
  ```

### 2. Фоновый Демон (Background Daemon)
Для работы в фоне без открытия терминала:
```bash
./ddb --daemon
# или
./ddb run
```

### 3. Установка в автозагрузку (systemd --user)
```bash
./ddb install
```
DDB автоматически создаст `~/.config/systemd/user/ddb-supervisor.service`, зарегистрирует его и запустит демон на порту `:8790`.

---

## 📡 REST & WebSocket API (`:8790`)

| Метод | Путь | Описание |
| :--- | :--- | :--- |
| `GET` | `/ping` | Быстрый liveness-чек сервиса |
| `GET` | `/status` | Телеметрия хоста (ОС, ЦПУ, ОЗУ, статус Hermes Desktop) |
| `POST` | `/exec` | Выполнение шелл-команд (если разрешено в матрице прав) |
| `POST` | `/action` | Универсальный диспетчер экшенов (`display_control`, `screen`, `fs.*`, `audio.play`) |
| `GET` | `/logs` | Кольцевой буфер последних событий |

---

## 📄 Лицензия

Распространяется под лицензией MIT. Разработано **Kasper Studios** (Kasperenok & Dirom).
