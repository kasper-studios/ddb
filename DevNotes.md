# DDB (Dirom Device Bridge) — Архитектурный Контракт и Спецификация

> **Авторы концепта:** Kasperenok & Dirom (Архитекторы)  
> **Исполнитель реализации:** Hermes (Инженер-Реализатор)  
> **Статус:** v1.3.0-GENERIC-ACTION-RPC  
> **Лицензия:** MIT / Суверенная Цитадель  

---

## 1. Фундаментальный Принцип: Supervisor-Centric API & Generic Action RPC

Никаких захардкоженных дублирующихся эндпоинтов вроде `/api/screen`, `/api/audio/play`, `/api/fs/read`!
Архитектура DDB строится на **Едином Универсальном Диспетчере Действий (Generic Action RPC Protocol)**:

```
                          ┌───────────────────────────┐
                          │   DIROM (Планшет / Агент) │
                          └─────────────┬─────────────┘
                                        │ POST /api/v1/action
                                        │ {"action": "screen_capture", "params": {...}}
                                        ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                    DDB SUPERVISOR RUNTIME (Порт :8790)                       │
│                         [Главный Служебный Сервер]                           │
│                                                                              │
│  - Единая точка входа API для Дирома                                         │
│  - Ультра-легковесный, неубиваемый демон автозагрузки (systemd)              │
│  - Эндпоинты:                                                                │
│      * GET  /api/v1/status  ➔ моментальный Heartbeat из памяти (15 мс)       │
│      * POST /api/v1/action  ➔ Generic RPC вход! Проксирует в Bridge по WS   │
│      * GET  /ws/events      ➔ стрим реалтайм-событий (Niri, Notifications)   │
│  - Watchdog & Keep-Alive воркера Bridge                                      │
│  - Аварийный Fallback: если Bridge упал/завис, экстренные базовые действия   │
│    ('exec', 'reboot', 'kill', 'restart_bridge') Супервайзер исполняет САМ!   │
└───────────────────────┬──────────────────────────────▲───────────────────────┘
                        │                              │
         IPC WebSocket  │ 1. {"id": 1, "action": "..."}│ 2. Heartbeat Telemetry &
         (ws://127.0.0.1)│    Проксирование в шелл     │    RPC Response
                        ▼                              │
┌──────────────────────────────────────────────────────┴───────────────────────┐
│                      DDB MAIN BRIDGE (Внутренний Воркер)                     │
│                        [Рабочий Адаптер Системы]                             │
│                                                                              │
│  - Запускается и контролируется Супервайзером                                │
│  - Подключен к графическому и звуковому окружению пользователя (Wayland/Niri)│
│  - Содержит таблицу зарегистрированных экшенов (Action Registry Map)         │
│  - Если запрошенный action не поддерживается хостом (например screen на VPS) │
│    или неизвестен ➔ Bridge сам возвращает:                                   │
│    {"ok": false, "error": "ACTION_NOT_SUPPORTED", "action": "..."}          │
│  - Эмитит непрерывный Heartbeat со своей телеметрией в Супервайзер           │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Спецификация Generic Action RPC (`POST /api/v1/action`)

Все операции (от выполнения bash-команд до захвата экрана и музыки) передаются через единый формат:

### 2.1. Формат Запроса (Request):
```json
{
  "action": "screen_capture",
  "params": {
    "quality": 80,
    "format": "jpeg"
  },
  "timeout_ms": 10000
}
```

### 2.2. Формат Успешного Ответа (Success Response):
```json
{
  "ok": true,
  "action": "screen_capture",
  "result": {
    "image_base64": "/9j/4AAQSkZJRg...",
    "mime": "image/jpeg",
    "width": 1920,
    "height": 1080
  },
  "duration_ms": 38
}
```

### 2.3. Формат Ошибки Неподдерживаемого Действия (Action Not Supported):
Если на безголовой VPS запросили скриншот или Niri, либо экшен опечатан:
```json
{
  "ok": false,
  "action": "screen_capture",
  "error": "ACTION_NOT_SUPPORTED",
  "message": "Display capability is disabled or unavailable on this host (headless environment)",
  "available_actions": [
    "shell_exec",
    "fs_read",
    "fs_write",
    "fs_list",
    "fs_delete",
    "audio_play",
    "mic_capture",
    "niri_focus",
    "window_list"
  ]
}
```

---

## 3. Таблица Базовых Экшенов (Standard Action Registry)

Каждый адаптер в Bridge просто регистрирует свои обработчики в реестр:
`registry.register('shell_exec', handleShellExec)`

| Action | Назначение | Параметры | Окружение |
| :--- | :--- | :--- | :--- |
| **`shell_exec`** | Выполнение команды с выводом | `{"command": "...", "cwd": "...", "timeout_ms": 30000}` | Все хосты |
| **`fs_read`** | Чтение файла с пагинацией | `{"path": "...", "offset": 1, "limit": 200}` | Все хосты |
| **`fs_write`** | Атомарная запись файла | `{"path": "...", "content": "..."}` | Все хосты |
| **`fs_list`** | Список файлов и директорий | `{"path": "...", "recursive": false}` | Все хосты |
| **`screen_capture`**| Мгновенный скриншот в память | `{"quality": 80, "format": "jpeg"}` | Только Desktop / Wayland |
| **`window_list`** | Список окон и их координаты | `{}` | Только Niri / Wayland |
| **`niri_action`** | Команда Niri IPC (`focus-window`, etc) | `{"msg": ["action", "focus-workspace", "1"]}` | Только Niri |
| **`audio_play`** | Воспроизведение звука в динамики | `{"url_or_path": "...", "volume": 1.0}` | PipeWire / ALSA |
| **`mic_capture`** | Захват отрезка аудио с микрофона | `{"duration_sec": 5, "format": "wav"}` | PipeWire / ALSA |
| **`system_reboot`**| Перезагрузка хоста | `{"delay_sec": 1}` | Все хосты (Fallback) |

Если модуль (например `screen`) не загрузился при старте из-за отсутствия Wayland — он просто **не попадает в реестр экшенов**. При попытке его вызвать Bridge мгновенно возвращает чистый `ACTION_NOT_SUPPORTED` с кодом 404/400 без крашей!

---

## 4. Heartbeat Telemetry & `device.status(name)`

Bridge непрерывно (каждые 2-4 секунды) эмитит в Супервайзер телеметрический пакет `heartbeat`:
```json
{
  "bridge_online": true,
  "bridge_uptime_sec": 4210,
  "system": {
    "hostname": "ponosik",
    "cpu_load_percent": 12.5,
    "memory_used_mb": 4120,
    "memory_total_mb": 15820,
    "battery_percent": 84,
    "is_charging": true,
    "temp_c": 46.2
  },
  "capabilities": {
    "display": true,
    "audio": true,
    "mic": true,
    "niri": true
  },
  "state": {
    "active_window": {
      "app_id": "firefox",
      "title": "YouTube - Mozilla Firefox"
    },
    "now_playing": {
      "player": "spotify",
      "title": "Клубим (KAKYN REMIX)",
      "artist": "5opka, MellSher"
    }
  },
  "supported_actions": [
    "shell_exec", "fs_read", "fs_write", "fs_list",
    "screen_capture", "window_list", "niri_action",
    "audio_play", "mic_capture"
  ]
}
```

Супервайзер держит этот объект в памяти на порту `:8790` (`GET /api/v1/status`), отдавая его агенту за **10-15 миллисекунд** без опроса ОС!

---

## 5. Категория инструментов `device.*` у Дирома

В ядре агента мы делаем красивый декларативный универсальный диспетчер:

* **`device.action(name, action, params={})`** — универсальный вызов ЛЮБОГО экшена на хосте;
* **`device.status(name)`** — мгновенный статус хоста (CPU, RAM, батарея, окно, плеер, доступные экшены);
* **Удобные нативные алиасы:**
  * `device.exec(cmd, name='ponosik')` ➔ `device.action(name, 'shell_exec', {'command': cmd})`
  * `device.screen(name='ponosik')` ➔ `device.action(name, 'screen_capture')`
  * `device.fs.read(path, name='ponosik')` ➔ `device.action(name, 'fs_read', {'path': path})`
  * `device.audio.play(sample, name='ponosik')` ➔ `device.action(name, 'audio_play', {'url_or_path': sample})`
  * `device.mic.listen(duration, name='ponosik')` ➔ `device.action(name, 'mic_capture', {'duration_sec': duration})`

---

## 6. Чеклист для Гермеса (Hermes Implementation Tasks)

- [ ] В `src/supervisor.js` реализовать:
  * `GET  /api/v1/status` (отдача закэшированного Heartbeat);
  * `POST /api/v1/action` (проксирование JSON-сообщения экшена в WebSocket Bridge и ожидание ответа по correlation ID);
  * Аварийный Fallback: если Bridge оффлайн/таймаут и экшен `shell_exec` — исполнять локально через `node:child_process`.
- [ ] В `src/bridge.js` реализовать:
  * Реестр экшенов `ActionRegistry` (словарь `name -> handler(params)`);
  * Если `action` не найден — возврат `ACTION_NOT_SUPPORTED` со списком `available_actions`;
  * Регистрация стандартных экшенов (`shell_exec`, `fs_*`, `screen_capture`, `audio_play`, `mic_capture`, `niri_*`);
  * Таймер отправки `heartbeat` каждые 3 секунды в Супервайзер.
- [ ] Протестировать вызов неизвестного экшена (например `foobar`) и убедиться, что Bridge возвращает корректную ошибку без краша.
