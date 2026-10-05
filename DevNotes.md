# DDB (Dirom Device Bridge) — Архитектурный Контракт и Спецификация

> **Авторы концепта:** Kasperenok & Dirom (Архитекторы)  
> **Исполнитель реализации:** Hermes (Инженер-Реализатор)  
> **Статус:** Draft v1.0.0-PROPOSAL  
> **Лицензия:** MIT / Суверенная Цитадель  

---

## 1. Концепция и Проблема

Текущий удалённый доступ к ноутбуку и серверам завязан на сырой SSH (`asyncssh`), что создаёт критические узкие места:
1. **Жесткая привязка к портам/IP:** необходимость прямого IP или проброса портов, чувствительность к разрывам соединения и NAT.
2. **Отсутствие реалтайм-событий:** SSH пассивен, он не умеет транслировать в модель системные эвенты хоста (смена активного окна в Niri, уведомления, состояние батареи, звук).
3. **Громоздкость:** исполнение сложных сценариев требует экранирования строк и парсинга неструктурированного текста bash.

**DDB (Dirom Device Bridge)** — универсальный легковесный агент хоста (Node.js ESM / TypeScript), работающий поверх протоколов HTTP REST + WebSocket + Cloudflare Tunnel, с авто-детекцией возможностей (Capability-Based Architecture).

---

## 2. Архитектура Возможностей (Capability-Based Architecture)

DDB обязан запускаться на **ЛЮБОМ** Linux-устройстве:
* Ноутбук `ponosik` (Arch Linux + Wayland / Niri WM);
* Облачные VPS (`truemain` / Ubuntu Headless без GUI и звука);
* Мобильные Linux-окружения (Termux / aarch64).

При старте DDB сканирует окружение и активирует флаги:

```json
{
  "hostname": "ponosik",
  "os": "Arch Linux 7.1.8-arch1-3",
  "arch": "x86_64",
  "capabilities": {
    "core": true,
    "display": true,
    "display_server": "wayland-niri",
    "audio": true,
    "audio_server": "pipewire",
    "notifications": true,
    "battery": true
  }
}
```

Если дисплей или звук отсутствуют (например, на VPS) — соответствующие модули мягко отключаются без падения демона.

---

## 3. Двухуровневая Структура Процессов

Для исключения ситуаций, когда сбой скрипта намертво вешает доступ к хосту, DDB разделен на **ДВА НЕЗАВИСИМЫХ ПРОЦЕССА**:

### Процесс 1: DDB Main Daemon (Порт `:8780`)
Основной богатый сервис с WebSocket и REST API.

#### Эндпоинты REST API:
* `GET /api/v1/ping` — телеметрия хоста (CPU, RAM, диски, uptime, capabilities).
* `POST /api/v1/exec` — запуск процесса.
  * *Request Body:* `{"command": "git status", "cwd": "/path", "timeout_ms": 30000, "env": {}}`
  * *Response:* `{"exit_code": 0, "stdout": "...", "stderr": "...", "duration_ms": 120}`
* `POST /api/v1/exec/stream` — SSE/Chunked запуск для длительных билдов (Gradle/Cargo).
* `GET /api/v1/screen` — мгновенный кадр экрана (только при `capabilities.display == true`).
  * Реализация под Niri/Wayland: вызов `grim -t jpeg -q 80 -` напрямую в ответ стримом без сохранения на SSD.
* `POST /api/v1/fs/read` — чтение файла с пагинацией (offset/limit строк).
* `POST /api/v1/fs/write` — атомарная запись файла.
* `POST /api/v1/fs/list` — просмотр директории с размерами и датами.
* `POST /api/v1/audio/play` — воспроизведение PCM/WAV/MP3 через `pw-cat -p` / `mpv` (при наличии audio).

#### WebSocket Event Stream (`/ws/events`):
Реалтайм-шина системных событий. Сообщения имеют вид:
```json
{
  "type": "niri_window_focus",
  "timestamp": 1791219800123,
  "data": {
    "app_id": "firefox",
    "title": "YouTube - Mozilla Firefox",
    "workspace": 1
  }
}
```
* **Источники событий:**
  * `niri`: чтение stdout от `niri msg -j event-stream`;
  * `notifications`: D-Bus мониторинг `org.freedesktop.Notifications`;
  * `mpris`: D-Bus отслеживание медиаплееров (`org.mpris.MediaPlayer2`);
  * `power`: UPower / sysfs события зарядки и батареи.

---

### Процесс 2: Emergency Fallback API (Порт `:8790`)
Аварийный минималистичный агент.
* **Стек:** Нативный микро-сервис без сторонних библиотек (чистый встроенный модуль `node:http`).
* **Размер кода:** < 70 строк.
* **Функционал:** строго один эндпоинт `POST /emergency/exec` с авторизацией по `X-DDB-Emergency-Token`.
* **Назначение:** если Main Daemon упал, завис в утечке памяти или зациклился — аварийный агент гарантированно отвечает и позволяет выполнить:
  * `systemctl --user restart ddb-main`
  * `kill -9 <pid>`
  * `reboot`

---

## 4. Сетевая Инфраструктура и Cloudflared

1. **Встроенная поддержка Cloudflared Tunnel:**
   * DDB содержит конфиг и сервис-скрипт для привязки к `cloudflared`.
   * Маршрутизация:
     * `*.kasperstudios.xyz` ➔ проксирует порт `:8780` (Main API & WSS);
     * Аварийный порт `:8790` может быть привязан к отдельному пути или открыт локально/через резервный токен.
2. **Безопасность (Zero-Trust Token):**
   * Все запросы требуют заголовок `Authorization: Bearer <DDB_SECRET_TOKEN>`.
   * Токен генерируется при первичной установке инсталлятором и сохраняется в защищенном конфиге `~/.config/ddb/config.json` (chmod 600).

---

## 5. Скрипт Инсталлятора (`install.sh`)

Установка в 1 команду:
```bash
curl -sSL https://raw.githubusercontent.com/kasper-studios/ddb/main/install.sh | bash
```

### Задачи инсталлятора:
1. Проверка наличия Node.js 20+ (при отсутствии — подсказка или авто-установка).
2. Клонирование репозитория в `~/.local/share/ddb` или `~/.config/ddb`.
3. Установка легковесных зависимостей (`npm ci --omit=dev`).
4. Генерация криптографического токена доступа.
5. Проверка установленных утилит хоста (`grim`, `pw-cat`, `cloudflared`, `niri`).
6. Создание и включение systemd user-юнитов:
   * `ddb.service` (Основной мост)
   * `ddb-emergency.service` (Аварийный шелл)
7. Вывод красивого ASCII-баннера DDB с короной и реквизитами подключения.

---

## 6. Задачи для Гермеса (Hermes Implementation Checklist)

- [ ] Создать структуру проекта (Node.js ESM, package.json, src/).
- [ ] Реализовать модуль детекции окружения `src/capabilities.js`.
- [ ] Реализовать `src/emergency_server.js` (изолированный `:8790`).
- [ ] Реализовать основной REST сервер Express/Fastify (`:8780`).
- [ ] Реализовать WebSocket брокер `src/ws_events.js` с адаптерами Niri IPC и D-Bus.
- [ ] Написать `install.sh` для развертывания на Arch/Ubuntu.
- [ ] Инициализировать Git-репозиторий и подготовить к пушу на GitHub.
