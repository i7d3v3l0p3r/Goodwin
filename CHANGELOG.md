# Changelog

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
