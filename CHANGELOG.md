# Changelog

## [Unreleased]

### English

- Install or update Goodwin from the terminal with `goodwin-installer install <address>`. You can select a local package, an ADB executable, and a wake-word model. The command prints the panel URL when the installation succeeds.
- Configure the assistant's STT → LLM → TTS engine in the panel. Each stage can use a separate provider and model, with additional parameters such as the synthesis voice. Enter a model ID manually when the server has no catalog.
- Adjust speech speed from 0.5× to 2× in the assistant's synthesis settings when the selected model supports it.
- With a streaming OpenAI-compatible server, Goodwin starts playing spoken replies as PCM arrives. Turn off streaming in **Settings → Assistant → Speech synthesis (TTS)** to wait for a complete reply before playback.
- OpenRouter is now a separate plugin. The OpenAI-compatible provider connects Goodwin to local or remote servers such as KittenTTS. Set the OpenRouter API key in the plugin after upgrading from the built-in provider.
- Goodwin renders plugin settings from their manifests. Open them from the Plugins list or the plugin's own page.
- Set separate microphone gains for wake-word detection and speech recognition. Speech recognition still supports automatic gain.
- On Station Mini 3, eyes blink while Goodwin records and the light ring spins while it processes a request. The four lower display segments brighten and fade with the spoken reply. Goodwin redraws the eyes after a clock refresh and restores the complete clock display at the end of the interaction.

### Русский

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
