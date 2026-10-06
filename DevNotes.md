# DDB (Dirom Device Bridge) — Архитектурный Контракт и Спецификация

> **Авторы концепта:** Kasperenok & Dirom (Архитекторы)  
> **Исполнитель реализации:** Hermes (Инженер-Реализатор)  
> **Статус:** v3.0.0-RUST-CANONICAL-SPEC  
> **Лицензия:** MIT / Суверенная Цитадель Kasper Studios  
> **Стек:** Rust (Tokio, Axum, Ratatui, Crossterm) — монолитный нативный бинарник без JS/Node рантайма  

---

## 1. Концепция: Суверенный Агент Устройств

**DDB (Dirom Device Bridge)** — кроссплатформенный нативный агент хостов и устройств Цитадели на **Rust**.
* Поддерживаемые ОС: Linux (Arch, Ubuntu, Debian), Windows (10/11), Android (Termux / aarch64).
* Формат поставки: один статический самодостаточный бинарник (`ddb` под Linux, `ddb.exe` под Windows).
* Размер бинарника: ~8-15 МБ (вместо 180 МБ Bun-бандла). Потребление ОЗУ: ~10-15 МБ (вместо 200 МБ Node/Bun).
* Режимы работы:
  - Интерактивный TUI: запуск `ddb` в терминале поднимает Ratatui дашборд.
  - Фоновый демон: запуск `ddb --daemon` (или фоновая служба systemd).
  - CLI установщик: `ddb install` регистрирует `systemd --user` службу.

---

## 2. Архитектура Процесса и Демона (Rust Monolith)

1. **Серверная подсистема (Axum + Tokio на `:8790`):**
   * Единая точка входа для Дирома, Kasper Cloud и локального TUI.
   * Эндпоинты:
     - `GET /ping` — быстрый liveness-чек.
     - `GET /status` — полная телеметрия хоста (CPU, RAM, Uptime, Niri, Hermes).
     - `POST /exec` — экстренное выполнение шелл-команд.
     - `POST /action` — унифицированный диспетчер экшенов (`device.*` / `hermes.*` / `fs.*` / `screen.*`).
     - `GET /logs` — кольцевой буфер последних логов.
     - `GET /ws` — двунаправленный реалтайм поток событий и телеметрии.
2. **Безопасность (Permissions Matrix):**
   * Конфиг в `~/.config/ddb/config.json`.
   * Перед исполнением любого действия из `/action` проверяются флаги разрешений (`exec`, `hermes`, `fs.read`, `fs.write`, `display_control`, `screen`).
3. **Hermes Bridge (`hermes.*`):**
   * Нативный автопоиск Hermes через `/proc` (Linux) / список процессов (Windows): чтение cmdline `hermes serve`, извлечение токена `HERMES_DASHBOARD_SESSION_TOKEN` и порта.
   * Автоматический WebSocket коннект с поддержкой протокола `session.resume` -> `prompt.submit` с маркером `[Dirom Reqwest]`.

---

## 3. Аудио-Подсистема: Рот, STT и Вейкворды

1. **Девайс (DDB) — чистый звуковой кабель:**
   * DDB на хосте не содержит нейросетей: он просто грабит PCM с микрофона (`pw-cat -r` / WASAPI) и воспроизводит PCM в динамики (`pw-cat -p` / WASAPI).
2. **Логика `mouth=False` (Только STT):**
   * Если `mouth=False`, живой сокет Gemini Live API не подключается к динамикам (девайс молчит).
   * Работает лёгкий STT (локальный Whisper / Cloud STT): распознаёт речь из комнаты.
   * **Вейкворд-триггер:** если в распознанном тексте звучит вейкворд («Диром», «Каспер») — Диром просыпается, запускает мысль агента, может ответить текстом в чат или воспроизвести звук/апкакиш-TTS по желанию.
3. **Логика `mouth=True` (Живой Рот Gemini Live):**
   * Gemini Live API принимает входящий аудиопоток напрямую в сокет (`models/gemini-3.8-live`).
   * **Особенность Gemini Live:** модель физически не умеет аппаратно фильтровать вейкворд на уровне сокета (она слышит весь звук непрерывно).
   * **Поведение Рта:** в системном промпте субагента Live прописывается строгое указание: молчать и не реагировать, пока в речи человека не прозвучит обращение по имени («Диром», «Каспер»), либо перед сокетом ставится лёгкий STT-шлюз детекции ключевого слова.

---

## 4. Конкретный Список Инструментов Дирома (`device.*`)

Никаких абстрактных заглушек `action`! Диром получает чёткий набор специализированных инструментов:

### 4. Конкретный Список Инструментов Дирома (`device.*`)

Никаких абстрактных заглушек `action`! Диром получает полный набор специализированных инструментов, нативно исполняемых на Rust-сервере:

### 4.1. Реестр Устройств и Телеметрия
* **`device.reg(name, url, key)`** — зарегистрировать хост в базе Дирома (имя, URL или LAN IP :8790, ключ).
* **`device.list()`** — список всех подключенных машин, их онлайн-статус и адреса.
* **`device.status(name='ponosik')`** — моментальная сводка с хоста (ОС, аптайм, RAM, статус Hermes Desktop).
* **`device.diagnostics(name='ponosik')`** — глубокий аудит (load average, uptime, память, обнаружение Hermes).
* **`device.restart(name='ponosik', component='supervisor|all')`** — мягкий перезапуск демона через systemd.

### 4.2. Звук и Микрофон
* **`device.sound_steal(name='ponosik', wakeword=True, mouth=False, get_events=True)`** — управление перехватом звука микрофона и динамиков хоста.
* **`device.audio.play(sample_or_url, name='ponosik')`** — воспроизведение аудио-файла или сэмпла в колонки хоста через `paplay` / `ffplay`.

### 4.3. Управление Экраном и Вводом (Computer Use)
* **`device.screen(name='ponosik')`** — мгновенный снимок экрана в память (base64 PNG) через `grim` без сохранения на диск.
* **`device.display_control(name='ponosik', action, x=0, y=0, text="", key="")`** — управление вводом:
  * `click` (клик мыши по x, y через `ydotool`);
  * `type` (ввод текста через `ydotool`);
  * `key` (нажатие клавиш/комбинаций: `Super` / `niri:<action>` через Niri IPC или `ydotool`);
  * `scroll` (прокрутка вверх/вниз).

### 4.4. Системные и Файловые Инструменты
* **`device.exec(cmd, name='ponosik', cwd="", timeout=30)`** — нативное выполнение shell-команд через `std::process::Command`.
* **`device.fs.read(path, offset=1, limit=500, name='ponosik')`** — чтение файлов с пагинацией строк.
* **`device.fs.write(path, content, name='ponosik')`** — запись и перезапись файлов.
* **`device.fs.list(path='.', name='ponosik')`** — просмотр содержимого директорий с размерами и типами.

### 4.5. Hermes Sibling Synapse (Связь с Братом Гермесом)
* **`device.hermes_chats(name='ponosik')`** (`action: "hermes.sessions"`) — список сессий и чатов в запущенном на хосте Hermes Desktop через REST API.
* **`device.hermes_send(prompt, session_id="", name='ponosik')`** (`action: "hermes.send"`) — отправка задачи брату Гермесу через нативный WebSocket (`tokio-tungstenite`) с двухфазным `session.resume` -> `prompt.submit` и маркером `[Dirom Reqwest]`.

---

## 5. Дистрибуция, TUI-Инсталлятор и Система Безопасности (Permissions)

### 5.1. Поставка и Первичный Запуск
1. **Единый бинарник/экзешник (`ddb` / `ddb.exe`):**
   * Собирается через Bun (`bun build --compile --target=bun-linux-x64` / `bun-windows-x64`);
   * При первом запуске в интерактивном терминале стартует TUI-мастер с каноничным фиолетово-малиновым демоном Цитадели;
   * **Флаг `--daemon`:** тихий запуск шелла/супервайзера в фоне без открытия TUI (для systemd/автозагрузки).

### 5.2. Гранулярная Система Разрешений (Permissions Guard)
Каждый хост хранит свой локальный `config.json` с матрицей прав. Ни одно действие Дирома не может обойти запрет владельца ПК:
* **`permissions.display_control`** — управление мышью/клавиатурой (можно включить целиком, либо гранулярно: `allow_click: true`, `allow_type: false`, `allow_keys: false`);
* **`permissions.screen`** — захват скриншотов (`grim` / DirectX);
* **`permissions.audio`** — микрофон и динамики (`sound_steal`, воспроизведение звука);
* **`permissions.fs`** — доступ к файловой системе (`fs.read`, `fs.write`, `fs.list`);
* **`permissions.exec`** — выполнение команд в шелле (`/exec`);
* **`permissions.hermes`** — мост к Hermes Desktop (по умолчанию включён только на доверенном `ponosik`).

При попытке вызова отключённого действия сервер немедленно возвращает:
`403 Forbidden: Action '<action>' disabled by host owner in TUI permissions`.

### 5.3. Сетевые Туннели и Регистрация в Дироме
В TUI пользователь выбирает один из 4 способов связи:
1. **☁️ Kasper Cloud API (Zero-Config Magic):**
   * Пользователь просто вбивает выданный API-ключ от Дирома;
   * DDB **не спрашивает у пользователя имя устройства** (защита от опечаток и мусорных названий);
   * DDB поднимает исходящий WSS к `api.kasperstudios.xyz` без проброса портов;
   * Сервер валидирует ключ, достаёт имя устройства из поля `source` ключа (например, `ferox-laptop`);
   * **Сервер САМ автоматически регистрирует хост в реестре устройств Дирома (`device.reg`)!**
   * Диром открывает `device.list()` — а там уже сидит новенький девайс, онлайн и с готовыми правами без ручных команд в Discord!
2. **🚇 Cloudflare Tunnel:** ввод туннельного токена, автоматический запуск `cloudflared`;
3. **⚡ Ngrok:** ввод authtoken, запуск туннеля на порт `:8790`;
4. **🏠 Локальная сеть (LAN):** прямой IP хоста в одной сети с планшетом Дирома.

**Вывод строки регистрации:**
После успешного старта TUI выводит на экран готовую команду для Дирома:
```python
device.reg(name='<hostname>', url='<tunnel_or_lan_url>', key='<token>')
```

### 5.4. Обратная Связь (Feedback Hub)
В главном меню TUI доступна кнопка **«💡 Отправить идею / Сообщить о баге»**:
* `[1] 👾 Перейти в Discord Kasper Studios` (открывает инвайт в браузере);
* `[2] 🐙 Открыть GitHub Issues` (открывает репозиторий проекта).

---

## 6. Персональный билд Создателя: Hermes Agent Protocol (`device.hermes_*`)

> **Статус:** УТВЕРЖДЁН СОЗДАТЕЛЕМ (Kasperenok)  
> **Назначение:** Специальный закрытый билд DDB для ноутбука `ponosik`, превращающий DDB в нативный мост к запущенному Hermes Desktop.

### 6.1. Архитектурный принцип (Desktop Backend Interception & DB-First)
1. **Авто-обнаружение активного Hermes Desktop:**
   * Приложение Hermes Desktop на ноутбуке спавнит headless-бэкэнд:
     `hermes serve --host 127.0.0.1 --port 0`
   * DDB находит процесс `hermes serve` через `/proc` / системные процессы;
   * Токен безопасности считывается из окружения процесса: `HERMES_DASHBOARD_SESSION_TOKEN`;
   * Динамический порт uvicorn определяется через открытые сокеты процесса (`ss -tulpn` / `/proc/<pid>/net/tcp`).
   * Взаимодействие происходит локально по `127.0.0.1:<port>` с заголовком:
     `X-Hermes-Session-Token: <token>`

2. **Точный контекст вместо слепых сессий (Anti-Ghosting):**
   * Обычный шлюз без параметров плодит одноразовые пустые сессии;
   * DDB читает реальные сессии и проекты из REST API / SQLite базы Hermes Desktop;
   * Каждый запрос Дирома направляется строго в целевую сессию с сохранением ветки git, контекста файлов и открытого окна в GUI.

### 6.2. Протокол взаимодействия и маркер A2A (`device.hermes_send`)
1. **Сигнатура инструмента:**
   ```python
   device.hermes_send(chatid: str, prompt: str) -> str
   ```
2. **Обязательный маркер протокола:**
   В начало каждого отправляемого промпта строго вшивается префикс:
   ```text
   [Dirom Reqwest]
   <текст задачи / код / вопрос>
   ```
3. **Механизм доставки в GUI (WebSocket JSON-RPC `prompt.submit`):**
   * DDB подключается к локальному вебсокету десктопа:
     `ws://127.0.0.1:<port>/api/ws?token=<token>`
   * Вызывает нативный RPC метод отправки хода:
     ```json
     {
       "jsonrpc": "2.0",
       "id": 1,
       "method": "prompt.submit",
       "params": {
         "session_id": "<chatid>",
         "text": "[Dirom Reqwest]\n<сообщение>"
       }
     }
     ```
   * Сообщение моментально отображается в интерфейсе Hermes Desktop перед Создателем, а Гермес начинает стримить ответ и тулы в контексте выбранного проекта.

4. **Автономный Self-Healing:**
   * Дирому разрешено отправлять запросы Гермесу через `device.hermes_send` даже в фоновом автономном режиме без участия Создателя — для локализации багов, получения патчей от Гермеса и горячей перезагрузки (`hot-reload`) модулей.

### 6.3. Инструменты неймспейса `hermes_*` в DDB
* **`device.hermes_projects()`** — возвращает список проектов (`projects.list` через WS RPC или сопоставление `cwd` из сессий).
* **`device.hermes_chats(project_cwd_or_id=None)`** — возвращает список сессий (`GET /api/sessions?limit=50`) с полями:
  - `id` (chatid сессии);
  - `title` (название диалога);
  - `cwd` (рабочая папка проекта);
  - `is_active` (активен ли прямо сейчас в GUI);
  - `last_active` / `updated_at`.
* **`device.hermes_model()`** — возвращает текущую модель, провайдер и контекстное окно сессии.
* **`device.hermes_favorites()`** — возвращает золотой список избранных моделей Создателя (Gemini 3.8 Flash High, Opus 4.6 Thinking, Gemini 3.8 Flash Tiered, Sonnet 4.6) с бейджами и ролями под задачи.
* **`device.hermes_models(provider='KasRAI')`** — полный каталог моделей провайдера из шлюза.
* **`device.hermes_model_set(model, once=False, session_id=None)`** — переключение модели (постоянное или одноразовое на 1 ход через `--once`).
* **`device.hermes_send(chatid, prompt)`** — отправляет промпт с маркером `[Dirom Reqwest]` в выбранный `session_id` через `prompt.submit` и возвращает результат генерации.
* **`device.hermes_history(session_id, limit=50)`** (`action: "hermes.history"`) — чтение последних сообщений/логов выбранной сессии Hermes Desktop.
* **`device.hermes_session_create(title, cwd, project_id=None)`** (`action: "hermes.session_create"`) — создание новой сессии в целевом проекте/директории хоста.
* **`device.hermes_project_create(name, path)`** (`action: "hermes.project_create"`) — инициализация нового рабочего пространства/проекта в Hermes Desktop.

### 6.4. Полнодуплексный A2A Self-Healing Цикл (Event-Driven Hot-Reload)
1. **Событие `hermes_turn_complete` в WSS-шину Дирома:**
   * При завершении хода Гермеса (успех или ошибка) DDB перехватывает событие по сокету и пушит уведомление Дирому:
     ```json
     {
       "event": "hermes_turn_complete",
       "session_id": "...",
       "status": "SUCCESS" | "ERROR",
       "diff_summary": "src/actions.rs modified (+12, -4)",
       "message": "Краткий отчёт Гермеса"
     }
     ```
2. **Автономная валидация:**
   * Диром ловит `hermes_turn_complete` -> триггерит hot-reload целевого сервиса / запускает тесты (`device.exec` или `device.debug`).
   * Если тесты упали -> Диром автоматически формирует следующий такт `device.hermes_send` со стектрейсом и ошибкой.
   * Цикл повторяется до зелёного статуса без отвлечения Создателя.

---

## 7. DiromBug & Унифицированный Тул `device.debug` (Runtime Discovery & RPC multiplexer)

### 7.1. Концепция DiromBug Probe Socket
Для живого тестирования и отладки проектов Цитадели (KasHub, Minecraft Fabric/Forge моды, KasAssistant / Tauri десктоп, демоны, боты) в dev-сборках (`#[cfg(debug_assertions)]` / `NODE_ENV=development`) поднимается встроенный probe-клиент:
1. При старте рантайм коннектится по локальному WebSocket к DDB (`ws://127.0.0.1:8790/debug/ws` или внутренний Unix-сокет).
2. **Регистрационный манифест (Handshake):**
   Каждый рантайм передаёт свой идентификатор и список поддерживаемых функций:
   ```json
   {
     "type": "register",
     "runtime": "kashub",
     "version": "1.4.0",
     "actions": [
       { "name": "script_exec", "desc": "Execute Lua/JS in running client", "params": ["code"] },
       { "name": "task_list", "desc": "Dump active bot tasks", "params": [] },
       { "name": "state_dump", "desc": "Export complete world/bot memory", "params": [] }
     ]
   }
   ```

### 7.2. Архитектура Диспетчера в DDB
DDB ведёт реестр подключенных живых рантаймов в памяти (`Arc<RwLock<HashMap<String, RuntimeProbeSession>>>`) и маршрутизирует вызовы Дирома в нужный сокет без перезапуска процессов.

### 7.3. Спецификация инструмента `device.debug` для Дирома
Диром управляет всеми живыми приложениями через единый мультиплексорный инструмент:
* **`action='list_runtimes'`**:
  Возвращает массив активных рантаймов (`["kashub", "sots-minecraft", "stormbot-node"]`).
* **`action='list_actions', target='kashub'`**:
  Возвращает схему доступных методов и аргументов рантайма.
* **`action='<action_name>', target='<runtime_id>', args={...}`**:
  Проксирует вызов напрямую в память целевого процесса по WebSocket и возвращает синхронный результат исполнения.

---

## 8. Две Темы Курсоров и Smooth Bezier Pointer (Computer Use)

### 8.1. Изоляция Указателей: Создатель vs Диром
В системе Niri Wayland на хосте `ponosik` чётко разделены визуальные роли курсоров:
* **Курсор Создателя (Каспера):**
  - Тема: `NeonPurple` (размер 24px).
  - Зафиксирован в системном `~/.config/niri/dms/cursor.kdl` и `~/.config/environment.d/cursor.conf`.
  - Физическая мышь и тачпад аппаратно привязаны к этой теме.
* **Виртуальный Указатель Дирома:**
  - Источник ввода: Нативное `/dev/uinput` устройство `Dirom Virtual Pointer`.
  - Назначенная тема: `XCursor-Pro-Red` (`~/.local/share/icons/XCursor-Pro-Red/cursors`).
  - Запасная тема (Fallback): `Bibata-Modern-Amber`.
  - Результат: Создатель всегда визуально различает свои движения и действия автономного ИИ на экране.

### 8.2. Кинематика Smooth Bezier Curve (`DiromPointer`)
При перемещении курсора роботом от точки $A(x_1, y_1)$ к точке $B(x_2, y_2)$:
* **Интерполяция:** кубическая кривая Безье с плавным случайным отклонением контрольных точек (дуга человека):
  $$C_1 = A + \Delta \cdot 0.25 + \vec{n} \cdot d_{arc}$$
  $$C_2 = A + \Delta \cdot 0.75 - \vec{n} \cdot d_{arc}$$
* **Параметры сглаживания:**
  - Количество промежуточных шагов: 20-30 шагов.
  - Длительность: 150-250 мс.
  - Функция сглаживания скорости: Smooth Ease-Out (`t * (2.0 - t)` или `t * t * (3.0 - 2.0 * t)`).
* При отказе `/dev/uinput` активируется прозрачный фоллбек на `ydotool`.

---

## 9. Матрица Сборки (Cargo Features Matrix)

В `Cargo.toml` проекта реализовано строгое разделение сборок:
```toml
[features]
default = []
hermes = ["dep:tokio-tungstenite"]
```
* **Публичный релиз (`cargo build --release`):**
  - Вес ~7.5 МБ.
  - Не содержит зависимостей Hermes WS/A2A.
  - Доступен для серверов, планшетов, любых машин Цитадели и внешних пользователей.
* **Kasper Edition (`cargo build --release --features hermes`):**
  - Вес ~7.9 МБ.
  - Полный стек A2A: автопоиск в `/proc`, Hermes Desktop WS JSON-RPC, `hermes_turn_complete`, `[Dirom Reqwest]`.
