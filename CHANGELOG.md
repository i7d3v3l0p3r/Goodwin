# Changelog

## [Unreleased]

### English

- Search model catalogs by name or ID and long language and provider lists by typing. Page changes, menus and settings sections now have short transitions that respect reduced-motion preferences.
- Now includes a mini chat for recent spoken and written exchanges. Expand system messages for timings, actions and failures of the last eight requests, or edit and repeat the latest voice command. The chat header can hide all system messages. The compact send control leaves the placeholder centered. Old chat links open Now. Active plugin cards appear first, and plugins can be pinned to the menu in each browser.
- Plugins can contribute standard or wide cards to the responsive Now grid. The compact Spotify player puts cover art, track details and playback control in a single row.
- Timers 0.2 can extend an active timer by one minute and remember a spoken phrase for its original duration. Activation settings include a three-attempt wake-word check.
- The chat offers editable example questions. The Station overview highlights the wake word, the assistant settings show the three voice stages, and the plugin catalogue groups actions with each package.
- A native Spotify plugin can search for music by voice and play the result on a configured Soloist receiver.
- Open Spotify from the desktop or mobile menu to pair a speaker, see the current track, browse upcoming and previous tracks, and control playback.
- Choose speech recognition and reply languages independently in the Mini 3 and ESP32-S3 panels. Both default to Russian, including on upgrade. The reply language applies to voice and text chat.

### Русский

- В каталогах моделей появился поиск по названию и ID, в длинных списках языков и провайдеров тоже можно искать. Страницы, меню и разделы настроек открываются плавнее и учитывают системную настройку уменьшения движения.
- На странице «Сейчас» появился мини-чат с голосовыми и текстовыми репликами. Системные сообщения раскрывают длительность этапов, действия плагинов и ошибки последних восьми запросов. Последнюю голосовую команду можно исправить и повторить. Системные сообщения можно скрыть переключателем в шапке. Кнопка отправки стала компактнее, а текст поля ввода выровнен по центру строки. Старые ссылки на чат открывают «Сейчас». Активные карточки плагинов показываются первыми, а нужные плагины можно закрепить в меню своего браузера.
- Плагины могут добавлять обычные и широкие карточки в адаптивную сетку «Сейчас». В компактном плеере Spotify обложка, название трека и кнопка воспроизведения занимают одну строку.
- Таймеры 0.2 позволяют добавить минуту к отсчёту и сохранить голосовую фразу для запуска таймера исходной длительности. В настройках активации появилась проверка слова в три попытки.
- В чате появились примеры вопросов, которые можно отредактировать перед отправкой. На экране колонки выделено слово активации, в настройках ассистента видны три этапа голосового ответа, а в каталоге действия собраны на карточке каждого плагина.
- Нативный плагин Spotify ищет музыку по голосовому запросу и запускает её на выбранном плеере Soloist.
- Из меню панели на компьютере и телефоне можно открыть Spotify. На странице есть подсказка по привязке колонки, текущий трек, очередь, история и управление воспроизведением.
- На Мини 3 и ESP32-S3 в панели можно отдельно выбрать язык распознавания речи и язык ответов. По умолчанию, в том числе после обновления, оба языка — русский. Язык ответа действует в голосовом диалоге и текстовом чате.

## [0.6.0] - 2026-11-27

### English

- The project overview is available in English and Russian, with panel screenshots in both languages and guides for each platform.
- On phones, open Chat, Wi-Fi or Station, and Plugins from the bottom bar. Menu brings settings within thumb reach. Larger plugin lists support search by name or page.
- Mini 3 and ESP32-S3 remember recent exchanges in a voice conversation. In activation settings, choose one request per wake word or button press, or enable follow-ups with a wait of 1–60 seconds after each answer (eight by default).
- GitHub releases now distribute Timers, OpenAI-compatible and Wake-word dataset only as WASM packages.
- New panel passwords use PBKDF2-HMAC-SHA512 with 220,000 iterations. Existing PBKDF2 passwords continue to work, and ESP32-S3 verifies saved SHA256 hashes faster.
- ESP32-S3 creates and verifies SHA512 panel password hashes faster and gives the watchdog time between batches of iterations.
- Panel sessions last 30 days. You can continue without a password during setup or change access in panel settings. Password length is shown in characters.
- ESP32-S3 speeds up dense and depthwise INT8 convolutions in the wake detector.
- ESP32-S3 uses microWakeWord by default for «Гудвин». Existing catalog selections remain available. Wake-word dataset 0.1.2 records the new 30 ms frames.

### Русский

- У проекта появились русский и английский обзоры со снимками панели на каждом языке и ссылками на инструкции для обеих платформ.
- На телефоне чат, Wi-Fi или колонка и плагины открываются с нижней панели. Остальные настройки доступны через «Меню». В большом списке плагинов можно искать по названию плагина или его страницы.
- На Мини 3 и ESP32-S3 Гудвин помнит предыдущие реплики голосового диалога. В настройках активации можно выбрать одну реплику по слову или кнопке либо диалог с продолжением. Время ожидания после ответа настраивается от 1 до 60 секунд, по умолчанию — 8.
- В релизах GitHub плагины таймеров, OpenAI-совместимого провайдера и датасета слова активации теперь доступны только в формате WASM.
- Новые пароли панели используют PBKDF2-HMAC-SHA512 с 220 000 итераций. Сохранённые пароли PBKDF2 подходят, а ESP32-S3 быстрее проверяет хеши SHA256.
- ESP32-S3 быстрее создаёт и проверяет хеши паролей SHA512 и даёт сторожевому таймеру время между пакетами итераций.
- Сессия панели действует 30 дней. Можно продолжить без пароля при первом запуске или изменить доступ в настройках панели. Длина пароля указана в символах.
- ESP32-S3 ускоряет обычные и поканальные свёртки INT8 в детекторе активации.
- ESP32-S3 по умолчанию распознаёт «Гудвин» через microWakeWord. Сохранённые модели из каталога остаются доступны. Датасет слова активации версии 0.1.2 записывает новые кадры по 30 мс.

## [0.5.0] - 2026-11-26

### English

- Mini 3 decodes buffered speech directly into its playback buffer. Mini 3 and ESP32-S3 reuse the conversation and tool definitions across calls within a voice request.
- ESP32-S3 keeps wake capture positions across long listening sessions and resynchronizes before using a prediction whose pre-roll was lost.
- ESP32-S3 diagnostics report how much audio reaches the wake detector and the largest delay behind captured audio.
- ESP32-S3 plays due timer alarms during pauses in streamed speech and then resumes the reply.
- Mini 3 packages cross-compiled on Windows consistently link the patched WAMR runtime.
- ESP32-S3 can play timer alarms while Wi-Fi reconnects or applies new settings.
- Mini 3 can be built with only the Micro INT8 wake runtime. On a fresh installation from this package, the installer selects the Micro INT8 model by default.
- On ESP32-S3, a voice or chat request can wait for the HTTPS clock while the device continues to handle due timer alarms.
- Streaming chat uses less temporary memory when decoding provider responses. The JSON parser scans long runs of plain text and Base64 in groups of bytes.
- WASM plugins allocate less temporary memory when reading small settings and dataset files.
- ESP32-S3 restores interrupted WASM plugin data writes before the plugin starts, including files accessed through the plugin storage API.
- The panel can export diagnostic counters and latencies as JSON. The export contains numeric metrics only.
- An open panel refreshes device capabilities after reconnecting and while it stays open. ESP32-S3 clients can read a completed wake model operation more than once.
- Timers on Mini 3 and ESP32-S3 preserve their countdown through wall-clock corrections and retry an unconfirmed alarm after restart.
- Provider model catalogs are processed incrementally. Speech playback rejects incompatible PCM metadata before decoding audio.
- Plugin installs and updates are confirmed after startup. A failed update restores the previous package and data. On ESP32-S3, other plugins can keep running during the update.
- Fixed memory alignment in the WASM and TensorFlow Lite Micro runtimes on Mini 3 and ESP32-S3.
- On Mini 3, the panel reports wake model installation by operation ID. Goodwin saves the active selection after the detector switches to it and resumes staged models after restart.
- Mini 3 and ESP32-S3 keep the selected voice providers and connection settings for the duration of a request. The panel can stop an active voice request during recording, provider calls, or playback.
- Changing audio settings on ESP32-S3 preserves the current microphone mute state.
- The ESP32-S3 panel can answer other requests while checking a password or downloading a wake model. API clients can poll `/api/auth/result` or `/api/wake-models/result` with the operation ID returned by the initial request.
- ESP32-S3 streams firmware and WASM package uploads in a background task. The panel can answer other requests while receiving either upload.
- ESP32-S3 can process due timers while waiting for a voice provider's response.
- Mini 3 tests the installed wake model with the incoming binary before replacing the service. On Stations with `flock`, the desktop installer recovers interrupted installations before selecting a wake model for the next attempt, even if the interruption occurs before the journal is created.
- Cancelling streamed speech lets native plugins finish the RPC briefly, then restarts them if the stream stays open.
- The wake-word dataset has a configurable storage limit and displays the space used. The panel respects the storage cap on ESP32-S3. Lowering the limit preserves existing recordings.
- The panel reuses cached gzip assets when their contents match the device's current build.

### Русский

- Мини 3 декодирует буферизованную озвучку сразу в буфер воспроизведения. Мини 3 и ESP32-S3 повторно используют историю диалога и описания инструментов при последовательных вызовах в одном голосовом запросе.
- ESP32-S3 продолжает захват при длительном прослушивании и отбрасывает результаты активации, у которых аудио до срабатывания уже вытеснено.
- Диагностика ESP32-S3 показывает объём захвата для детектора активации и максимальное отставание от принятого звука.
- ESP32-S3 воспроизводит сигналы таймеров в паузах потоковой озвучки и затем продолжает ответ.
- При сборке пакетов Мини 3 на Windows в исполняемый файл попадает WAMR с исправлениями.
- ESP32-S3 воспроизводит сигналы таймеров во время переподключения Wi-Fi и применения новых сетевых настроек.
- Мини 3 можно собрать только с движком активации Micro INT8. При первой установке из такого пакета установщик по умолчанию выбирает модель Micro INT8.
- На ESP32-S3 запрос чата или голосовой запрос может ждать синхронизации часов для HTTPS, пока устройство воспроизводит сигналы таймеров.
- Потоковый чат использует меньше временной памяти при разборе ответов провайдера. JSON-парсер проверяет длинные участки обычного текста и Base64 группами байтов.
- WASM-плагины используют меньше временной памяти при чтении небольших настроек и файлов датасета.
- ESP32-S3 восстанавливает файлы после прерванной записи до запуска WASM-плагина. Это касается и файлов, доступных через API хранилища плагинов.
- Из панели можно скачать счётчики и задержки в формате JSON. В выгрузку входят только числовые показатели.
- Открытая панель обновляет список возможностей устройства после переподключения и во время работы. На ESP32-S3 результат установки модели активации можно запросить повторно.
- Таймеры на Мини 3 и ESP32-S3 сохраняют ход отсчёта при коррекции системных часов и повторяют неподтверждённый сигнал после перезапуска.
- Гудвин разбирает каталоги моделей провайдеров по мере загрузки. Озвучивание отклоняет несовместимые параметры PCM до декодирования звука.
- Установка или обновление плагина подтверждается после запуска. При ошибке обновления Гудвин восстанавливает предыдущий пакет и его данные. На ESP32-S3 остальные плагины могут работать во время обновления.
- Исправлено выравнивание памяти в WASM и TensorFlow Lite Micro на Мини 3 и ESP32-S3.
- На Мини 3 панель показывает результат установки модели активации по ID операции. Гудвин сохраняет активный выбор после переключения детектора и продолжает установку подготовленной модели после перезапуска.
- Мини 3 и ESP32-S3 фиксируют выбор голосовых провайдеров и параметры подключений на весь запрос. Из панели можно остановить активный голосовой запрос во время записи, обращения к провайдеру или озвучки.
- Изменение аудионастроек на ESP32-S3 сохраняет текущее состояние переключателя микрофона.
- Панель ESP32-S3 отвечает на другие запросы во время проверки пароля или загрузки модели активации. Клиенты API получают ID операции и запрашивают результат через `/api/auth/result` или `/api/wake-models/result`.
- ESP32-S3 принимает прошивку и WASM-пакеты порциями в фоновой задаче. Во время загрузки панель отвечает на другие запросы.
- ESP32-S3 может обрабатывать таймеры, пока ждёт ответа голосового провайдера.
- Мини 3 проверяет установленную модель активации новой сборкой перед заменой службы. На Станциях с `flock` установщик обрабатывает прерывание до создания журнала и восстанавливает установку до выбора модели для следующей попытки.
- После отмены потоковой озвучки нативный плагин получает короткую паузу на завершение запроса. Если поток остаётся открыт, Goodwin перезапускает плагин.
- В датасете слова активации можно задать лимит хранилища и видеть занятый объём. Панель учитывает предел хранилища ESP32-S3. Уменьшение лимита сохраняет существующие записи.
- Панель повторно использует gzip-файлы из кэша, если их содержимое совпадает с текущей сборкой устройства.

## [0.4.0] - 2026-11-25

### English

- Reduced the memory used to format diagnostic entries on Mini 3 and ESP32-S3 and to retain them on ESP32-S3.
- Reduced the size of the ESP32-S3 firmware and bundled WASM provider and wake-word dataset plugins.
- Goodwin uses a shared typed JSON codec for panel requests, saved settings, and WASM host services.
- Provider chat messages, tool declarations and model parameters use typed JSON. The assistant settings expose named model options. Known options from saved settings are loaded and fields outside the new schema are skipped.
- Updated the Rust dependencies of Goodwin, its bundled plugins and the desktop installer.
- Goodwin reads the current assistant configuration format and addresses named providers as `<plugin-id>:<connection-id>`. Re-enter API settings from standalone provider packages in the unified plugin and reselect any stage still using an earlier provider ID.
- Use `goodwin wav` to process a WAV file from the command line.
- Mini 3 and ESP32-S3 build the WAV for speech recognition in the existing capture buffer.
- New wake-word dataset clips include the gain used by the wake detector. The export identifies which clips have gain applied to their audio.
- The standalone OpenRouter plugin was removed. Use the OpenRouter connection in the OpenAI-compatible plugin.
- The Station installer uses wake models from the catalog. Select a new catalog model when updating a Station that used a model bundled with an older installer or whose saved selection lacks a runtime.
- ESP32-S3 and Mini 3 use PBKDF2-HMAC-SHA256 for panel passwords. To replace an earlier password hash on Mini 3, stop Goodwin, run `goodwin set-password <data-dir>`, then restart. On ESP32-S3, provision Wi-Fi over USB again and set a new panel password.
- Goodwin selects the platform, device model and drivers at build time. `goodwin device-capabilities` reports the Mini 3 build's activation modes, wake runtimes and speech streaming support.
- The standalone ESP32-S3 N16R8 build supports local «Гудвин» activation, push-to-talk and an OpenAI-compatible voice pipeline with buffered and streaming playback. Its web panel configures Wi-Fi and the provider, runs chat and voice diagnostics, and installs firmware updates.
- The ESP32-S3 panel installs WASM add-ons, including Timers, Wake-word dataset and OpenAI-compatible. Plugin packages and data persist in a separate Flash partition. The voice pipeline can select plugin providers independently for each stage.
- ESP32-S3 and Station Mini 3 use WAMR Fast for WASM add-ons. Installed packages and their data carry over when upgrading from Wasmi.
- The ESP32-S3 panel saves a custom system prompt and lets signed-in users read the current boot's diagnostic log.
- On ESP32-S3, choose a compatible Micro INT8 wake model from Goodwin-wakeword and run a wake-word voice check while continuous listening is paused.
- The Station release package is smaller because the Goodwin binary ships without its symbol table.
- Chat comes first in the main navigation and opens from the Goodwin logo. Timers has a direct navigation link when installed. The gear menu opens below its button. The setup wizard shows one step at a time, opens at the next check when the assistant is already connected, and can be skipped in the current browser.
- Setup checks now identify the tested models and ask you to repeat a check after changing its configuration. Voice diagnostics link failures to the relevant settings.
- The installer shows recovery actions for common failures and reports the version returned by the installed Goodwin binary.
- The desktop installer checks Station compatibility and ADB access before installation, exposes its controls to screen readers, and reports installation stages. The panel opens Goodwin setup after you create the password.
- Goodwin setup guides you through provider configuration and text and voice checks. The panel gives Station, Chat, Assistant, and Add-ons their own navigation entries, including on phones.
- Chat keeps drafts while navigating, blocks sending when a provider needs configuration, and accounts for microphone mute in its wake-word prompt.
- The panel shows which assistant stages use a connection before removal, confirms updates before restarting Goodwin, and lets signed-in users copy diagnostic logs.
- Timers marks stale data and confirms cancellation. The wake-word dataset keeps its filter in the URL and protects unsaved score changes when leaving the page.

### Русский

- Снижено потребление памяти при форматировании записей журнала на Мини 3 и ESP32-S3 и при их хранении на ESP32-S3.
- Уменьшен размер прошивки ESP32-S3 и комплектных WASM-плагинов провайдеров и датасета слова активации.
- Goodwin использует общий типизированный JSON-кодек для запросов панели, сохранённых настроек и вызовов хоста из WASM.
- Сообщения чата, описания инструментов и параметры моделей используют типизированный JSON. Настройки ассистента показывают отдельные поля параметров. Goodwin загружает известные поля из сохранённых настроек и пропускает поля вне новой схемы.
- Обновлены зависимости Rust в Goodwin, комплектных плагинах и настольном установщике.
- Goodwin читает текущий формат настроек ассистента. ID именованных подключений имеют вид `<ID плагина>:<ID подключения>`. Перенесите настройки API из отдельных плагинов в объединённый и заново выберите подключения для стадий со старыми ID.
- Для обработки WAV из командной строки используется команда `goodwin wav`.
- Мини 3 и ESP32-S3 собирают WAV для распознавания речи в буфере записи.
- Новые записи датасета слова активации сохраняются с усилением детектора. В экспорт добавлена отметка о том, применено ли усиление к записи.
- Отдельный плагин OpenRouter удалён. Используйте подключение OpenRouter в OpenAI-совместимом плагине.
- Установщик Станции использует модели активации из каталога. При обновлении Станции со встроенной моделью из старого пакета или с сохранённой моделью без указания рантайма выберите модель из каталога заново.
- ESP32-S3 и Мини 3 используют PBKDF2-HMAC-SHA256 для пароля панели. Чтобы заменить прежний хеш на Мини 3, остановите Goodwin, задайте пароль командой `goodwin set-password <data-dir>` и снова запустите службу. На ESP32-S3 заново настройте Wi-Fi через USB и задайте пароль панели.
- Платформа, модель устройства и драйверы выбираются при сборке Goodwin. Команда `goodwin device-capabilities` показывает режимы активации, рантаймы слова активации и поддержку потокового звука в сборке для Мини 3.
- Автономная прошивка для ESP32-S3 N16R8 поддерживает локальную активацию по «Гудвин», активацию кнопкой и голосовой каскад через OpenAI-совместимый API с потоковым воспроизведением. В панели можно настроить Wi-Fi и провайдера, проверить чат и голосовой запрос и обновить прошивку.
- В панели ESP32-S3 можно установить WASM-дополнения, включая «Таймеры», «Датасет слова активации» и OpenAI-совместимый провайдер. Пакеты и данные хранятся в отдельном разделе флеш-памяти. Провайдер плагина можно выбрать отдельно для каждой стадии голосового каскада.
- На ESP32-S3 и Станции Мини 3 WASM-дополнения работают через WAMR Fast. Установленные пакеты и их данные сохраняются при обновлении с Wasmi.
- В панели ESP32-S3 можно сохранить свой системный промпт, а после входа с паролем доступен журнал диагностики текущего запуска.
- На ESP32-S3 можно выбрать совместимую модель Micro INT8 из Goodwin-wakeword и запустить голосовую проверку по слову активации при выключенном постоянном прослушивании.
- Пакет для Станции стал меньше за счёт удаления таблицы символов из файла `goodwin`.
- «Чат» стоит первым в основной навигации и открывается по логотипу Goodwin. Когда плагин таймеров установлен, в меню есть прямая ссылка «Таймеры». Меню настроек открывается под шестерёнкой. Мастер показывает по одному шагу, сразу переходит к проверке настроенного ассистента и запоминает пропуск в текущем браузере.
- В настройке Goodwin видны проверенные модели. При изменении конфигурации панель предлагает повторить тест. Голосовая диагностика ведёт из ошибок к нужным настройкам.
- Установщик предлагает действия для типовых сбоев и показывает версию, которую сообщила установленная сборка Goodwin.
- Установщик проверяет совместимость Станции и доступ через ADB до установки, предоставляет элементы управления экранным дикторам и показывает этапы установки. Панель открывает настройку Goodwin после создания пароля.
- Настройка Goodwin помогает подключить провайдера и проверить текстовый и голосовой ответы. «Колонка», «Чат», «Ассистент» и «Дополнения» доступны из основной навигации, в том числе на телефоне.
- Чат сохраняет черновик при переходе между страницами, блокирует отправку при ненастроенном провайдере и учитывает отключение микрофона в подсказке активации.
- Панель показывает, какие этапы ассистента используют подключение, запрашивает подтверждение перед перезапуском при обновлении и позволяет копировать журналы после входа с паролем.
- Плагин «Таймеры» отмечает устаревшие данные и запрашивает подтверждение отмены. Датасет слова активации сохраняет фильтр в URL и предупреждает об уходе со страницы с несохранённым порогом.

## [0.3.1] - 2026-11-24

### English

- Timers is available as a WASM package and retains saved timers when upgrading from the native package.
- The Timers WASM package is smaller with `no_std` and link-time optimization.
- OpenAI-compatible manages multiple named provider connections, including OpenRouter. Existing provider selections and credentials carry over when the unified WASM package is installed.
- After migration, standalone OpenRouter settings open the unified provider settings. Deleting the migrated connection leaves the other connections available.
- Pending WASM provider requests are cancelled when Goodwin stops. Updates also wait for older Goodwin builds to finish stopping when their first stop attempt times out.
- WASM provider connections share Goodwin's HTTP client and support streaming. The Wake-word dataset WASM package keeps recordings in its existing data directory.
- Station builds run WASM plugins through Wasmi with per-plugin memory and execution limits. WASM plugins can register pages, HTTP routes and assistant tools.
- Goodwin embeds precompressed panel assets and serves them as gzip.
- An optional Hotpath build profiles Rust allocations by function and thread on the Station.
- Station builds can run Micro INT8 wake-word models through TensorFlow Lite Micro. The panel and installer check the model runtime against the Goodwin build.
- Micro wake-word models use one inference graph and reuse the recurrent state tensors between audio frames.
- The panel receives status and chat updates as they happen. Replies from OpenRouter and streaming OpenAI-compatible servers appear as the model generates them.
- Plugins can choose an icon for each page listed in the sidebar and mobile menu. Timers and Wake-word dataset have their own icons.
- The **Wake-word dataset** plugin collects audio windows and model scores while Goodwin listens. Set a minimum score for collection, listen to and label recordings, download labelled WAVs with their metadata, or clear all recordings from the panel.
- The Goodwin service writes a timestamped journal with startup, microphone, plugin, and request events. Failed requests include the stage and provider when available. The journal rotates automatically and is visible in **Settings → Diagnostics**.
- **Settings → Diagnostics** shows what Goodwin heard and answered in its latest voice request, with stage timings, models and errors. The page also shows system and Goodwin resource usage on Linux and gives signed-in users access to recent kernel and Goodwin logs.
- Check for and install Goodwin updates from **Settings → Goodwin update** on a Station set up with the installer. Goodwin verifies the release, restarts, and restores the previous version if the new service fails its startup check.

### Русский

- Плагин «Таймеры» доступен в WASM и сохраняет таймеры при обновлении с нативного пакета.
- Размер WASM-пакета «Таймеров» уменьшен за счёт `no_std` и LTO.
- OpenAI-compatible поддерживает несколько именованных подключений, включая OpenRouter. При установке объединённого WASM-пакета сохраняются ключи и выбранные в ассистенте подключения.
- После миграции настройки отдельного OpenRouter открывают страницу объединённого провайдера. Удаление перенесённого подключения не прерывает работу остальных.
- При остановке Goodwin отменяет ожидающие HTTP-запросы WASM-провайдеров. Если старая версия не остановилась с первой попытки, установщик дожидается её завершения.
- WASM-подключения провайдеров используют общий HTTP-клиент Goodwin и поддерживают потоковые ответы. WASM-пакет датасета слова активации сохраняет записи в том же каталоге данных.
- Сборки для Станции запускают WASM-плагины через Wasmi с лимитами памяти и выполнения для каждого плагина. WASM-плагины могут регистрировать страницы, HTTP-маршруты и инструменты ассистента.
- Goodwin встраивает заранее сжатые файлы панели и отдаёт их в gzip.
- Диагностическая сборка с Hotpath показывает аллокации Rust по функциям и потокам на Станции.
- Сборка для Станции поддерживает модели активации Micro INT8 через TensorFlow Lite Micro. Панель и установщик проверяют, поддерживает ли сборка нужный рантайм модели.
- Модели активации Micro используют один граф вычислений и переиспользуют тензоры состояния между аудиокадрами.
- Панель получает изменения состояния и чата сразу. Ответы OpenRouter и OpenAI-совместимых серверов с поддержкой стриминга появляются по мере генерации.
- Плагины могут выбирать иконку для каждой страницы в боковом и мобильном меню. У «Таймеров» и «Датасета слова активации» свои иконки.
- Плагин **Датасет слова активации** сохраняет аудиофрагменты и оценки модели во время прослушивания. В панели можно задать минимальную оценку для записи, прослушать и разметить фрагменты, скачать размеченные WAV с метаданными или удалить все записи.
- Сервис Goodwin ведёт журнал с отметками времени для запуска, микрофона, плагинов и запросов. При сбое видны этап и провайдер, если он участвовал в запросе. Журнал автоматически ротируется и доступен в разделе **Настройки → Диагностика**.
- В разделе **Настройки → Диагностика** можно посмотреть, что Гудвин услышал и ответил в последнем голосовом запросе, длительность этапов, модели и ошибки. Там же доступны показатели ресурсов системы и Goodwin на Linux, а после входа с паролем — свежие записи журналов ядра и Goodwin.
- Проверяйте и устанавливайте обновления Goodwin из раздела **Настройки → Обновление Goodwin** на Станции, настроенной установщиком. Goodwin проверяет релиз, перезапускается и восстанавливает предыдущую версию, если новая не проходит проверку запуска.

## [0.2.0] - 2026-11-24

### English

- First-run password setup shows its requirements and field errors. An empty chat offers a link to install, choose, or configure a provider when setup is needed.
- Settings, Plugins, and Timers use aligned content widths. Plugin rows put opening and configuring first, with removal in the actions menu. Active timers show a larger countdown.
- Chat keeps your place while you read earlier messages and offers a jump to the latest reply. Waiting and response errors appear beside the message they belong to.
- Station diagnostics show the current voice test, its stages, and the latest completed test. The test result stays visible after unrelated chat or voice requests.
- Assistant and plugin settings use shared disclosures and save controls with progress and result messages. Plugin removal asks for confirmation in a dialog that names the package and shows any removal error.
- Timers and Plugins use shared status, error, and empty states. Timers show a loading status until the first response and clear connection errors when polling succeeds again.
- Assistant settings and Timers share button and field styles. The assistant and wake-word settings share the same switches, and pressing Enter in the timer form starts the timer.
- The panel asks before clearing the chat or leaving unsaved settings. On mobile, plugin pages are listed in a scrollable navigation menu, and long titles and status messages fit beside the microphone controls.
- If saving wake-word sensitivity fails, the panel restores the saved value and offers to retry next to the slider.
- Mute or unmute Goodwin’s microphone from any page in the web panel. Muting stops active recording and blocks wake-word listening and voice tests. Goodwin remembers the setting after a restart.
- The web panel uses Lucide icons for navigation, chat, and settings.
- Choose a wake-word model from Goodwin-wakeword during installation or switch models without restarting in **Settings → Activation and microphone**. The panel confirms the change once the detector is using the new model. The Station package contains the service and launcher. Updates keep the selected model by default.
- The Station installer can unpack updates in temporary storage, reducing the free space they need on the Station.
- Settings now open to an overview with separate pages for activation and microphone, the assistant, Station testing, and the panel language. The assistant page shows the current provider and model for each stage before you expand its controls.
- Install or update Goodwin from the terminal with `goodwin-installer install <address>`. You can select a local package, an ADB executable, and a wake-word model. The command prints the panel URL when the installation succeeds.
- Configure the assistant's STT → LLM → TTS engine in the panel. Each stage can use a separate provider and model, with additional parameters such as the synthesis voice. Enter a model ID manually when the server has no catalog.
- Adjust speech speed from 0.5× to 2× in the assistant's synthesis settings when the selected model supports it.
- With a streaming OpenAI-compatible server, Goodwin starts playing spoken replies as PCM arrives. Turn off streaming in **Settings → Assistant → Speech synthesis (TTS)** to wait for a complete reply before playback.
- OpenRouter is now a separate plugin. The OpenAI-compatible provider connects Goodwin to local or remote servers such as KittenTTS. Set the OpenRouter API key in the plugin after upgrading from the built-in provider.
- Goodwin renders plugin settings from their manifests. Open them from the Plugins list or the plugin's own page.
- Set separate microphone gains for wake-word detection and speech recognition. Speech recognition still supports automatic gain.
- On Station Mini 3, eyes blink while Goodwin records and the light ring spins while it processes a request. The four lower display segments brighten and fade with the spoken reply. Goodwin redraws the eyes after a clock refresh and restores the complete clock display at the end of the interaction.

### Русский

- При первом запуске требования к паролю и ошибки видны у полей. Если для чата нужен провайдер, пустой экран ведёт к установке, выбору или настройке.
- Настройки, плагины и таймеры выровнены по общей ширине. В строках плагинов на виду действия «Открыть» и «Настроить», а удаление находится в меню. У активных таймеров выделен отсчёт.
- Чат сохраняет позицию при чтении истории и предлагает перейти к последним сообщениям. Ожидание ответа и ошибки видны рядом с сообщением, к которому относятся.
- Проверка колонки показывает ход и этапы голосового теста. Результат последней проверки остаётся на экране после других текстовых и голосовых запросов.
- В настройках ассистента и плагинов используются общие раскрывающиеся секции и кнопки с состоянием сохранения. Диалог удаления называет плагин и показывает ошибку, если удалить пакет не удалось.
- В таймерах и списке плагинов используются общие статусы, ошибки и пустые состояния. Таймеры показывают загрузку до первого ответа и убирают ошибку связи после успешного обновления.
- В настройках ассистента и таймерах используются общие кнопки и поля. В настройках ассистента и активации появились единые переключатели, а таймер можно запустить клавишей Enter в форме.
- Веб-панель запрашивает подтверждение перед очисткой чата или выходом из настроек с несохранёнными изменениями. На телефоне страницы плагинов собраны в прокручиваемом меню, а длинные заголовки и статусы помещаются рядом с управлением микрофоном.
- Если сохранить чувствительность активации не удалось, панель возвращает сохранённое значение и предлагает повторить попытку рядом с ползунком.
- Включайте и отключайте микрофон Гудвина с любой страницы веб-панели. Отключение прерывает запись, поиск слова активации и голосовые тесты. Настройка сохраняется после перезапуска.
- В навигации, чате и настройках веб-панели используются иконки Lucide.
- Выбирайте модель активации из каталога Goodwin-wakeword при установке или меняйте её без перезапуска в разделе **Настройки → Активация и микрофон**. Панель подтверждает смену, когда детектор уже использует новую модель. Пакет для Станции содержит сервис и скрипт запуска. При обновлении выбранная модель сохраняется по умолчанию.
- Установщик может распаковывать обновления во временном разделе. Обновлениям требуется меньше места на Станции.
- Настройки открываются с оглавления. Активация и микрофон, ассистент, проверка колонки и язык панели вынесены в отдельные разделы. В разделе ассистента провайдер и модель каждого этапа видны до раскрытия его настроек.
- Устанавливайте и обновляйте Goodwin из терминала командой `goodwin-installer install <адрес>`. Можно указать локальный пакет, путь к ADB и модель активации. После успешной установки команда выводит адрес панели.
- Настраивайте движок ассистента STT → LLM → TTS в панели. Для каждого этапа можно выбрать провайдер, модель и дополнительные параметры, например голос озвучивания. Если сервер не отдаёт каталог, ID модели можно ввести вручную.
- В настройках озвучивания можно менять скорость речи от 0,5× до 2×, если выбранная модель это поддерживает.
- С потоковым OpenAI-совместимым сервером Goodwin начинает озвучивать ответ по мере поступления PCM. Чтобы воспроизводить готовый ответ целиком, отключите потоковое воспроизведение в разделе **Настройки → Ассистент → Озвучивание (TTS)**.
- OpenRouter вынесен в отдельный плагин. OpenAI-совместимый провайдер подключает локальные и удалённые серверы, в том числе KittenTTS. При обновлении со встроенного OpenRouter укажите ключ API в настройках плагина.
- Goodwin строит формы настроек плагинов по их манифестам. Настройки открываются из списка плагинов или со страницы самого плагина.
- Усиление микрофона для ключевого слова и распознавания речи настраивается отдельно. Для распознавания доступно автоусиление.
- На Станции Мини 3 глаза моргают во время записи, а кольцо вращается при обработке запроса. При озвучивании четыре нижних сегмента дисплея плавно меняют яркость в такт голосу. Гудвин возвращает глаза после перерисовки часов и восстанавливает полный кадр часов по окончании запроса.

## [0.1.0] - 2026-11-23

### English

- The desktop installer for Windows, Linux and macOS finds prepared Yandex Station Mini 3 devices on the local network and installs or updates Goodwin. It verifies the downloaded package against `SHA256SUMS` and restores the previous installation if the updated service fails to start.
- Set a custom system prompt in the web panel to guide how Goodwin responds in chat and by voice.
- The Timers plugin now uses SolidJS for its panel interface. The Probe plugin is no longer shipped.

See the [desktop installer guide](https://github.com/i7d3v3l0p3r/Goodwin/blob/v0.1.0/README.md#desktop-installer).

### Русский

- Установщик для Windows, Linux и macOS находит подготовленные Яндекс Станции Мини 3 в локальной сети и устанавливает или обновляет Goodwin. Он сверяет скачанный пакет с `SHA256SUMS` и восстанавливает предыдущую установку, если после обновления сервис не запускается.
- В настройках веб-панели можно задать системный промпт для ответов Goodwin в чате и голосом.
- Интерфейс плагина «Таймеры» переведён на SolidJS. Плагин Probe исключён из поставки.

См. [инструкцию по работе с установщиком](https://github.com/i7d3v3l0p3r/Goodwin/blob/v0.1.0/README.md#desktop-installer) на английском.

## [0.0.1] - 2026-11-23

### English

First release of Goodwin, an alternative voice assistant for Yandex Station Mini 3 (`bergamot`). Goodwin runs alongside Alice on a prepared Station.

- Say **«Гудвин»** followed by a question. Goodwin detects the wake word locally and uses OpenRouter to transcribe the question, generate an answer, and speak it through the Station.
- Use the web panel to chat, configure OpenRouter, tune wake sensitivity, and test the microphone and speaker. Set the panel password on first visit. Signed-in sessions survive service restarts.
- Install plugins from the panel. Timers can be created, listed, and cancelled from the panel or by voice with an OpenRouter model that supports tool calling. The Probe plugin checks plugin installation and UI integration.
- Download `goodwin-mini3-aarch64.tar.gz` for the Station. It includes the Goodwin executable, a launcher, and the Standard and Micro wake-word models. The two plugin archives are separate downloads. Use `SHA256SUMS` to verify the files.

See the [installation guide](https://github.com/i7d3v3l0p3r/Goodwin/blob/v0.0.1/README.md#run-on-the-station).

### Русский

Первый релиз Goodwin, альтернативного голосового ассистента для Яндекс Станции Мини 3 (`bergamot`). Goodwin работает параллельно с Алисой на подготовленной Станции.

- Скажите **«Гудвин»** и сразу задайте вопрос. Станция распознаёт ключевое слово локально. Goodwin отправляет запись вопроса в OpenRouter и воспроизводит озвученный ответ через Станцию.
- В веб-панели можно общаться в чате, настроить OpenRouter, изменить чувствительность активации и проверить микрофон и динамик. При первом входе задайте пароль. Сессии сохраняются после перезапуска сервиса.
- Плагины устанавливаются через панель. Таймеры можно создавать, просматривать и отменять через панель или голосом, если модель OpenRouter поддерживает вызов инструментов. Плагин Probe позволяет проверить установку и работу интерфейса плагинов.
- Для Станции скачайте `goodwin-mini3-aarch64.tar.gz`. Внутри исполняемый файл Goodwin, скрипт запуска и модели активации Standard и Micro. Архивы двух плагинов доступны отдельно. Контрольные суммы файлов указаны в `SHA256SUMS`.

См. [инструкцию по установке](https://github.com/i7d3v3l0p3r/Goodwin/blob/v0.0.1/README.md#run-on-the-station) на английском.
