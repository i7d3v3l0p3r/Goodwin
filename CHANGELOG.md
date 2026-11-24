# Changelog

## [Unreleased]

## [0.3.0] - 2026-11-24

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
