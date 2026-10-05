# DDB (Dirom Device Bridge) — Архитектурный Контракт и Спецификация

> **Авторы концепта:** Kasperenok & Dirom (Архитекторы)  
> **Исполнитель реализации:** Hermes (Инженер-Реализатор)  
> **Статус:** v1.4.0-CRASH-SHIELD-DEFENSE  
> **Лицензия:** MIT / Суверенная Цитадель  

---

## 1. Фундаментальный Принцип: Crash Shield & Неубиваемый Шелл

Шелл Супервайзера (`:8790`) спроектирован так, чтобы он **ФИЗИЧЕСКИ НЕ МОГ УМЕРЕТЬ НИ ПРИ КАКИХ ОБСТОЯТЕЛЬСТВАХ**.
Это последняя, непробиваемая цитадель обороны хоста (Crash Shield):

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                    DDB SUPERVISOR RUNTIME (Порт :8790)                       │
│                        «КРАШ-ШИЛД» (CRASH SHIELD)                            │
│                                                                              │
│  - Написан на ультра-сыром, минималистичном node:http (< 80 строк кода)     │
│  - 0 внешних npm-зависимостей! Никаких express, ws или тяжелых библиотек!    │
│  - Не использует динамическую память и сложные структуры (иммунитет к OOM)   │
│  - Перехватывает uncaughtException и unhandledRejection на глобальном уровне  │
│  - Защищённый эндпоинт аварийного шелла: POST /api/v1/action ('shell_exec')  │
│  - ВСЕГДА отвечает, даже если вся графическая сессия или Wayland крашнулись! │
└───────────────────────┬──────────────────────────────▲───────────────────────┘
                        │                              │
         IPC WebSocket  │ 1. Проксирование экшенов     │ 2. Heartbeat Telemetry &
         (ws://127.0.0.1)│    в рабочую сессию          │    BlackBox Post-Mortem
                        ▼                              │
┌──────────────────────────────────────────────────────┴───────────────────────┐
│                      DDB MAIN BRIDGE (Внутренний Воркер)                     │
│                        [Рабочий Адаптер Системы]                             │
│                                                                              │
│  - Богатый функционал (Niri, Wayland, PipeWire, Screen, Audio, FS)           │
│  - Может быть подвержен сбоям ОС, падениям композитора или тяжелым нагрузкам │
│  - Непрерывно пишет свой стейт в кольцевой буфер Черного Ящика (BlackBox)    │
│  - При падении: Супервайзер перехватывает код завершения, сигнал (SIGSEGV/   │
│    SIGKILL), стек ошибки и фиксирует время даунтайма в журнал крушений       │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Защитные принципы Краш-Шилда:
1. **Zero External Dependencies:** Супервайзер использует строго стандартную библиотеку Node.js (`node:http`, `node:child_process`, `node:fs`). Никакой баг в `node_modules` не может его сломать.
2. **Изоляция сбоев:** Смерть Bridge (например, падение PipeWire или краш Niri) никак не задевает сокет Супервайзера.
3. **Прямой системный Fallback:** Если Bridge мёртв, `shell_exec` исполняется напрямую встроенным `node:child_process.spawn('/bin/bash')`. Связь с машиной гарантирована на 100%.

---

## 2. Посмертная Диагностика (Post-Mortem Diagnostics & Черный Ящик)

Супервайзер ведет журнал крушений в оперативной памяти и локальном файле `~/.config/ddb/crash_report.json`.

При падении Bridge Супервайзер фиксирует:
1. **Точный таймштамп аварии** и причину (код выхода `exitCode`, сигнал `SIGKILL`, `SIGSEGV`, `OOM`);
2. **Секундомер даунтайма** (`downtime_sec` — сколько секунд воркер находится в оффлайне);
3. **Последние 50 строк stderr/stdout** перед смертью (BlackBox ring-buffer);
4. **Статус целостности системы (Damage Assessment):**
   * Доступен ли Wayland сокет (`/run/user/1000/niri*.sock`);
   * Жив ли PipeWire;
   * Хватает ли оперативной памяти на хосте;
5. **Вердикт авто-восстановления:** возможно ли безопасно поднять Bridge, или требуется ручное вмешательство через аварийный шелл.

### JSON-срез диагностики (`GET /api/v1/diagnostics` или `device.status`):
```json
{
  "host": "ponosik",
  "shield_status": "ARMED_AND_ACTIVE",
  "bridge_state": "CRASHED",
  "downtime_sec": 42,
  "crash_report": {
    "crashed_at": "2026-10-05T17:05:12.100Z",
    "exit_code": 137,
    "signal": "SIGKILL",
    "reason": "OOM_KILLED",
    "last_error": "FATAL ERROR: Ineffective mark-compacts near heap limit Allocation failed - JavaScript heap out of memory",
    "recent_stderr_tail": [
      "[BRIDGE] Processing huge file...",
      "<--- Last few GCs --->",
      "[BRIDGE] FATAL ERROR: Reached heap limit"
    ]
  },
  "damage_assessment": {
    "niri_socket_available": true,
    "pipewire_available": true,
    "free_ram_mb": 11400,
    "can_auto_recover": true
  },
  "recovery_action": "RESTART_AVAILABLE"
}
```

---

## 3. Спецификация Generic Action RPC (`POST /api/v1/action`)

Все операции передаются через единый формат:

### 3.1. Формат Запроса (Request):
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

### 3.2. Формат Успешного Ответа:
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

### 3.3. Формат Ошибки Неподдерживаемого Действия:
```json
{
  "ok": false,
  "action": "screen_capture",
  "error": "ACTION_NOT_SUPPORTED",
  "message": "Display capability is disabled or unavailable on this host",
  "available_actions": [
    "shell_exec", "fs_read", "fs_write", "fs_list", "diagnostics"
  ]
}
```

---

## 4. Таблица Базовых Экшенов (Standard Action Registry)

| Action | Назначение | Параметры | Исполнитель при сбое Bridge |
| :--- | :--- | :--- | :--- |
| **`shell_exec`** | Запуск команды в bash | `{"command": "...", "cwd": "...", "timeout_ms": 30000}` | **Супервайзер (Fallback)** |
| **`diagnostics`** | Посмертная диагностика и аудит сбоя | `{}` | **Супервайзер (Нативно)** |
| **`bridge_restart`**| Принудительный перезапуск воркера | `{}` | **Супервайзер (Нативно)** |
| **`fs_read`** | Чтение файла с пагинацией | `{"path": "...", "offset": 1, "limit": 200}` | **Супервайзер (Fallback)** |
| **`fs_write`** | Атомарная запись файла | `{"path": "...", "content": "..."}` | **Супервайзер (Fallback)** |
| **`fs_list`** | Список файлов и директорий | `{"path": "...", "recursive": false}` | **Супервайзер (Fallback)** |
| **`screen_capture`**| Скриншот экрана в память | `{"quality": 80, "format": "jpeg"}` | Только живой Bridge (Wayland) |
| **`window_list`** | Список окон и их координаты | `{}` | Только живой Bridge (Niri) |
| **`audio_play`** | Воспроизведение звука в динамики | `{"url_or_path": "...", "volume": 1.0}` | Только живой Bridge (PipeWire) |
| **`mic_capture`** | Захват отрезка аудио с микрофона | `{"duration_sec": 5, "format": "wav"}` | Только живой Bridge (PipeWire) |
| **`system_reboot`**| Перезагрузка хоста | `{"delay_sec": 1}` | **Супервайзер (Нативно)** |

---

## 5. Категория инструментов `device.*` у Дирома

Диром получает полноценные инструменты управления и диагностики хоста:

* **`device.status(name='ponosik')`** — живой срез состояния (онлайн/оффлайн, батарея, окно Niri, плеер, аптайм);
* **`device.diagnostics(name='ponosik')`** — **полная диагностика краша:** когда упал, код завершения, стек ошибки, сколько секунд висит даунтайм, повреждены ли сокеты Wayland/PipeWire и можно ли поднять заново;
* **`device.restart(name='ponosik', component='bridge|all')`** — перезапуск через Супервайзер;
* **`device.exec(cmd, name='ponosik')`** — выполнение шелл-команд (работает ВСЕГДА, даже при мертвом Бридже через Краш-Шилд!);
* **`device.screen(name='ponosik')`** — скриншот экрана;
* **`device.fs.read / write / list`** — файловая система;
* **`device.audio.play` & `device.mic.listen`** — звук и микрофон.

---

## 6. Чеклист для Гермеса (Hermes Implementation Tasks)

- [ ] В `src/supervisor.js` реализовать **Crash Shield**:
  * Слушать порт `:8790` строго на нативном `node:http` (0 зависимостей);
  * Глобальные перехватчики `process.on('uncaughtException')` и `unhandledRejection` (процесс не падает никогда);
  * Ведение кольцевого буфера BlackBox (последние 50 строк вывода Bridge);
  * Детект падения дочернего процесса (`child.on('exit')`): фиксация кода выхода, сигнала и времени крушения;
  * Реализация эндпоинта `GET /api/v1/diagnostics` (полный аудит даунтайма и проверка Niri/RAM);
  * Аварийный fallback `shell_exec` через `node:child_process.spawn`.
- [ ] В `src/bridge.js` реализовать стандартный ActionRegistry и отправку хартбитов.
- [ ] Протестировать падение Бриджа по `kill -9`: убедиться, что Супервайзер на `:8790` остался жив, выдал точный отчет о крушении в `device.diagnostics` и по команде поднял Бридж обратно!
