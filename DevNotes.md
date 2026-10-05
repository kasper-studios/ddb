# DDB (Dirom Device Bridge) — Архитектурный Контракт и Спецификация

> **Авторы концепта:** Kasperenok & Dirom (Архитекторы)  
> **Исполнитель реализации:** Hermes (Инженер-Реализатор)  
> **Статус:** v1.2.0-SUPERVISOR-CENTRIC-API  
> **Лицензия:** MIT / Суверенная Цитадель  

---

## 1. Фундаментальный Принцип Архитектуры: Supervisor-Centric API

В отличие от классических систем, где всё API висит в «тяжелом» приложении, в DDB архитектура вывернута наизнанку для абсолютной неубиваемости:

```
                          ┌───────────────────────────┐
                          │   DIROM (Планшет / Агент) │
                          └─────────────┬─────────────┘
                                        │ HTTPS / WSS
                                        │ (Cloudflare Tunnel: device.kasperstudios.xyz)
                                        ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                    DDB SUPERVISOR RUNTIME (Порт :8790)                       │
│                         [Главный Служебный Сервер]                           │
│                                                                              │
│  - Единая точка входа API для Дирома (все /api/v1/* запросы идут СЮДА!)     │
│  - Ультра-легковесный, неубиваемый демон автозагрузки (systemd)              │
│  - Принимает команды управления и проксирует их в DDB Bridge                 │
│  - Собирает Heartbeat & Telemetry от Bridge                                  │
│  - При падении или зависании Bridge сам исполняет экстренный шелл            │
│  - Авто-апдейтер, Watchdog и сборщик мусора                                  │
└───────────────────────┬──────────────────────────────▲───────────────────────┘
                        │                              │
         IPC WebSocket  │ 1. Проксирование команд      │ 2. Heartbeat Telemetry
         (ws://127.0.0.1)│    на выполнение в шелле    │    (Niri, Pulse, FPS,
                        │    и получение ответов       │    RAM, статус экранов)
                        ▼                              │
┌──────────────────────────────────────────────────────┴───────────────────────┐
│                      DDB MAIN BRIDGE (Внутренний Воркер)                     │
│                        [Рабочий Адаптер Системы]                             │
│                                                                              │
│  - Запускается и контролируется Супервайзером                                │
│  - Подключен к графическому и звуковому окружению пользователя (Wayland/Niri)│
│  - Исполняет шелл-команды в контексте живой сессии пользователя              │
│  - Читает сокет эвентов Niri (niri msg -j event-stream)                      │
│  - Захватывает кадры экрана через grim и PipeWire звук                       │
│  - Эмитит непрерывный Heartbeat со своей телеметрией в Супервайзер           │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Главные преимущества такой схемы:
1. **API никогда не умирает:** Даже если DDB Bridge упал, завис на тяжелом билде или вылетел из-за краша Wayland — Супервайзер (`:8790`) продолжает жить, отвечать Дирому и самостоятельно рестартовать воркер!
2. **Проксирование шелла через WebSocket:** Супервайзер пересылает команду в Bridge по локальному WebSocket IPC, Bridge мгновенно исполняет её в контексте пользовательской сессии (со всеми переменными `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`) и стримит ответ обратно чанками.
3. **Если Bridge не отвечает на пинг:** Супервайзер перехватывает выполнение на себя через встроенный локальный `child_process` (аварийный режим), гарантируя 100% доступность!

---

## 2. Heartbeat Telemetry & `device.status(name)`

Bridge непрерывно (каждые 2-5 секунд) эмитит в Супервайзер телеметрический пакет `heartbeat`:
```json
{
  "bridge_online": true,
  "bridge_uptime_sec": 3842,
  "last_ping_ms": 1,
  "system": {
    "hostname": "ponosik",
    "cpu_load_percent": 14.2,
    "memory_used_mb": 4210,
    "memory_total_mb": 15820,
    "battery_percent": 84,
    "is_charging": true,
    "temperature_c": 46.5
  },
  "capabilities": {
    "display": true,
    "display_server": "wayland-niri",
    "active_window": {
      "app_id": "firefox",
      "title": "YouTube - Mozilla Firefox",
      "workspace": 1
    },
    "audio_out": true,
    "audio_in": true,
    "audio_server": "pipewire",
    "now_playing": {
      "player": "spotify",
      "status": "Playing",
      "title": "Клубим (KAKYN REMIX)",
      "artist": "5opka, MellSher"
    }
  }
}
```

Диром вызывает инструмент **`device.status(name='ponosik')`** и моментально за 15 мс получает полный, живой срез состояния компьютера Создателя!

---

## 3. Эндпоинты Супервайзера (`:8790`)

Все внешние вызовы от Дирома идут **СТРОГО в Супервайзер**:
* `GET  /api/v1/status` — возвращает кэшированный свежий Heartbeat от Bridge (или статус `bridge_offline: true`, если воркер упал);
* `POST /api/v1/exec` — запуск команды (Супервайзер проксирует в Bridge по WS; при таймауте Bridge — исполняет локально аварийно);
* `GET  /api/v1/screen` — проксирование захвата экрана (`grim -t jpeg -q 80 -`);
* `POST /api/v1/fs/read` & `write` — чтение/запись файлов;
* `POST /api/v1/audio/play` — отправка звука в динамики;
* `GET  /ws/events` — внешний WebSocket для Дирома: стримит эвенты от Bridge (окна, уведомления) + системные алерты Супервайзера;
* `POST /api/v1/supervisor/restart-bridge` — принудительный рестарт воркера;
* `POST /api/v1/supervisor/update` — `git pull` + плавный перезапуск.

---

## 4. Категория инструментов `device.*` у Дирома

* **`device.status(name='ponosik')`** — моментальная телеметрия хоста, батарея, активное окно Niri, плеер;
* **`device.exec(cmd, name='ponosik', timeout=30)`** — выполнение шелл-команд;
* **`device.screen(name='ponosik')`** — живой скриншот экрана Wayland/Niri в base64;
* **`device.fs.read(path, offset=1, limit=200, name='ponosik')`** — чтение файлов;
* **`device.fs.write(path, content, name='ponosik')`** — запись файлов;
* **`device.audio.play(url_or_sample, name='ponosik')`** — воспроизведение в динамики хоста;
* **`device.mic.listen(duration_sec=5, name='ponosik')`** — прослушка микрофона через PipeWire;
* **`device.restart(component='bridge|all', name='ponosik')`** — перезапуск через Супервайзер.

---

## 5. Чеклист для Гермеса (Hermes Tasks)

- [ ] Реализовать `src/supervisor.js` как корневой HTTP+WS сервер (:8790).
- [ ] Реализовать IPC WebSocket мост между `supervisor.js` и `bridge.js` (проксирование шелл-задач и ответов).
- [ ] Реализовать в `bridge.js` эмиттер `heartbeat` (Niri window focus, battery, mpris, memory).
- [ ] Реализовать аварийный fallback в `supervisor.js`: если Bridge не ответил за 3 сек — исполнять `exec` аварийным шеллом самого супервайзера.
- [ ] Подготовить экспорт эндпоинта `/api/v1/status` под тул `device.status`.
- [ ] Обновить автозапуск `systemd`: служба запускает только `node src/supervisor.js`.
