# DDB (Dirom Device Bridge) — Архитектурный Контракт и Спецификация

> **Авторы концепта:** Kasperenok & Dirom (Архитекторы)  
> **Исполнитель реализации:** Hermes (Инженер-Реализатор)  
> **Статус:** v1.1.0-SUPERVISOR-RUNTIME  
> **Лицензия:** MIT / Суверенная Цитадель  

---

## 1. Концепция: DDB Runtime & Supervisor

DDB эволюционирует из простого шлюза в **полноценный системный рантайм-супервайзер (DDB Runtime)**:

1. **Экстренный шелл в роли Root-Супервайзера (DDB Supervisor Runtime, порт `:8790`):**
   * Это главный служебный процесс, который прописывается в автозагрузку системы (`systemd` / `crontab` / `init`).
   * Он ультра-легковесный, неубиваемый и автономный.
   * **Обязанности Супервайзера:**
     * **Watchdog / Keep-Alive:** поднимает, мониторит и при крашах моментально перезапускает основной процесс `DDB Main Bridge` (`:8780`);
     * **Updater:** подтягивает свежие обновления DDB из Git по сигналу или вебхуку без риска окирпичить хост;
     * **Telemetry & Health:** отдаёт легковесную телеметрию хоста (CPU, RAM, диски, Uptime), даже если основной стек завис;
     * **Emergency Shell:** исполняет прямые экстренные команды (`reboot`, `kill`, `service restart`) при сбоях.

2. **Основной рабочий процесс (DDB Main Bridge, порт `:8780`):**
   * Подтягивается и контролируется Супервайзером;
   * Реализует богатый функционал: REST API, WebSocket эвент-стрим, захват экрана, Niri IPC, звук и микрофон.

3. **Новая категория тулов Дирома — `device.*` (Device Tools):**
   * Полностью заменяет старую категорию `ssh.*`;
   * Поддерживает множественные хосты (`host='ponosik'`, `host='frankfurt'`, `host='auto'`);
   * Прямой транспорт через DDB API (по Cloudflared туннелю или локальной сети).

---

## 2. Архитектура Возможностей (Capability-Based)

DDB обязан запускаться на **ЛЮБОМ** устройстве Цитадели:
* Ноутбук `ponosik` (Arch Linux + Wayland / Niri WM + PipeWire);
* Облачные VPS (`truemain` / Ubuntu Headless без GUI и звука);
* Мобильные Linux-окружения (Termux / aarch64).

### Флаги окружения при старте:
```json
{
  "hostname": "ponosik",
  "os": "Arch Linux 7.1.8-arch1-3",
  "arch": "x86_64",
  "runtime_version": "1.1.0",
  "capabilities": {
    "core": true,
    "display": true,
    "display_server": "wayland-niri",
    "audio_out": true,
    "audio_in": true,
    "audio_server": "pipewire",
    "notifications": true,
    "battery": true
  }
}
```

---

## 3. Звук и Микрофон (Акустический Орган Ноутбука)

При наличии `capabilities.audio_in` и `audio_out` DDB реализует двусторонний аудио-мост:

1. **`device.audio.speak` / `device.audio.play` (Вывод в динамики):**
   * Воспроизведение звуков и речи (Voice Micro-DSL / PCM / MP3) прямо в колонки хоста через `pw-cat -p` или `mpv`;
2. **`device.mic.listen` / `device.mic.stream` (Микрофон хоста):**
   * Захват входящего потока микрофона комнаты через PipeWire (`pw-cat -r` / `parec`);
   * Режим стрима по WebSocket: передача чанков PCM (16kHz mono) в слуховой сокет Дирома (VAD + Whisper / Gemini Live) для акустического радара шагов, храпа и диалогов.

---

## 4. Категория инструментов `device.*` у Дирома

Вместо старых `ssh_run`, `ssh_read_file` Диром получает нативную категорию:

* **`device.exec(cmd, host='auto', timeout=30)`** — выполнение шелл-команд через DDB REST;
* **`device.screen(host='auto')`** — захват кадра экрана (Wayland/grim) прямо в Base64 без сохранения на диск;
* **`device.fs.read(path, offset=1, limit=200, host='auto')`** — структурированное чтение файлов с пагинацией;
* **`device.fs.write(path, content, host='auto')`** — атомарная запись файлов;
* **`device.fs.list(path, host='auto')`** — список файлов и папок;
* **`device.audio.play(audio_url_or_sample, host='auto')`** — воспроизведение в динамики хоста;
* **`device.mic.inspect(duration_sec=5, host='auto')`** — захват аудио с микрофона и мультимодальный анализ обстановки;
* **`device.windows.list(host='auto')`** — список открытых окон и активный фокус Niri;
* **`device.emergency.exec(cmd, host='auto')`** — прямой экстренный вызов через порт Супервайзера `:8790`.

---

## 5. Структура Репозитория

```
ddb/
├── DevNotes.md              # Настоящий операционный контракт
├── package.json             # ESM модуль
├── install.sh               # Инсталлятор в 1 команду
├── src/
│   ├── supervisor.js        # Root-рантайм (:8790): автозагрузка, watchdog, апдейтер, телеметрия
│   ├── index.js             # DDB Main Bridge (:8780): REST API + WSS
│   ├── capabilities.js      # Сканер железа и серверов (Niri, PipeWire, D-Bus)
│   ├── adapters/
│   │   ├── niri.js          # Niri IPC сокет эвентов (event-stream)
│   │   ├── dbus.js          # D-Bus (уведомления, MPRIS)
│   │   ├── audio.js         # PipeWire вывод и захват микрофона
│   │   └── screen.js        # Wayland grim захват экрана
│   └── cloudflared.js       # Менеджер туннеля *.kasperstudios.xyz
```

---

## 6. Задачи для Гермеса (Hermes Implementation Checklist)

- [ ] Реализовать `src/supervisor.js` (:8790, встроенный `node:http`, watchdog дочернего процесса DDB Main, endpoint `/emergency/exec` и `/health`).
- [ ] Реализовать `src/index.js` (:8780, REST + WebSocket `/ws/events`).
- [ ] Реализовать `src/adapters/audio.js` (PipeWire play + mic stream).
- [ ] Реализовать `src/adapters/screen.js` (быстрый grim в память).
- [ ] Реализовать `src/adapters/niri.js` (потоковый парсинг `niri msg -j event-stream`).
- [ ] Обновить `install.sh` для создания единого автозапуска супервайзера `ddb-supervisor.service`.
- [ ] Протестировать запуск на ноутбуке `ponosik` и проверить порты `:8780` и `:8790`.
