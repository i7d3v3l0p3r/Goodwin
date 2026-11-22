export const words = {
  en: {
    chat: "Chat", settings: "Settings", newChat: "New chat", nav: "Goodwin navigation",
    empty: "What can I help you with?", sayOrWrite: "Say “Goodwin” or write a message below.",
    write: "Write a message below.", draft: "Message Goodwin", send: "Send", you: "You",
    ready: "Listening for Goodwin", paused: "Listening paused", offline: "Station microphone unavailable",
    noModel: "Wake word unavailable", disconnected: "Reconnecting…", connecting: "Connecting…",
    account: "Enter the panel password.", password: "Password", login: "Sign in", logout: "Sign out",
    setupAccount: "Create a password for the web panel.", confirmPassword: "Confirm password", createPassword: "Create password",
    preferences: "Preferences", language: "Language", listening: "Listen for “Goodwin”",
    systemPrompt: "System prompt",
    systemPromptHint: "Describe Goodwin’s role, personality and tone in chat and voice replies. Leave blank to use the default.",
    systemPromptExample: "You are Goodwin, a calm home assistant with a dry sense of humor. Keep replies short and friendly.",
    voiceCheck: "Test the Station", testHint: "Speak near the Station. The reply plays through its speaker.",
    manual: "Start with a button", byWake: "Start with “Goodwin”", last: "Last request",
    noRequest: "The result and timings will appear here.", completed: "Completed", ms: "ms",
    provider: "OpenRouter", key: "API key", keySaved: "A key is configured.", keyEmpty: "Add a key to send requests.",
    replaceKey: "Enter a new key", removeKey: "Remove saved key", save: "Save changes", saved: "Saved",
    transcription: "Speech recognition model", chatModel: "Answer model", speech: "Speech synthesis model",
    voice: "Voice", sensitivity: "Wake word sensitivity", advanced: "Models and sensitivity",
    plugins: {
      title: "Plugins", empty: "No plugins installed.", upload: "Upload package",
      archive: "Choose a .tar.gz package to install or update.", remove: "Remove", cancel: "Cancel",
      confirm: "Remove this plugin and all its saved data?", password: "Enable password access to manage plugin packages.",
      missing: "This plugin page is unavailable.", loading: "Loading plugin…", loadError: "Could not load the plugin page.",
      status: { starting: "Starting", ready: "Ready", updating: "Updating", removing: "Removing", error: "Unavailable" },
    },
    stages: {
      awaiting_wake: "Say “Goodwin” near the Station", recording: "Listening", transcribing: "Recognizing speech",
      thinking: "Preparing an answer", synthesizing: "Preparing playback", playing: "Playing an answer",
    },
    errors: {
      busy: "Goodwin is already handling a request.", microphone: "The Station microphone is unavailable.",
      model: "The wake word model is unavailable.", missing_key: "Add an OpenRouter API key in Settings.",
      cloud: "OpenRouter could not complete the request.", playback: "The Station could not play the answer.",
      no_speech: "Could not make out the request.", no_wake: "The wake word was not heard.",
      wrong_password: "Check the password.", login_retry: "Wait a moment and try again.",
      invalid_password: "Use a password between 8 and 256 bytes.", password_mismatch: "The passwords do not match.",
      setup_complete: "A password has already been set. Sign in to continue.", password_save_failed: "Could not save the password. Try again.",
      save_failed: "Could not save the settings.", invalid_settings: "Check the model names, voice and API key.",
      invalid_system_prompt: "Enter a system prompt of up to 4,000 characters.",
      invalid_message: "Enter a message of up to 2,000 characters.",
      connection: "Goodwin is unreachable. Try again when it reconnects.", internal: "Something went wrong.",
      invalid_plugin_package: "The archive is invalid or incompatible with this device.", plugin_archive_too_large: "The archive exceeds 16 MiB.",
      plugin_password_required: "Enable password access to manage plugin packages.", plugin_not_found: "The plugin could not be found.",
      plugin_start_failed: "The plugin failed to start.", plugin_update_rolled_back: "The update failed. The previous version and its data were restored.",
      plugin_storage_failed: "Goodwin could not save or restore the plugin files.", plugin_unavailable: "The plugin is unavailable.",
    },
  },
  ru: {
    chat: "Чат", settings: "Настройки", newChat: "Новый чат", nav: "Навигация Goodwin",
    empty: "Чем могу помочь?", sayOrWrite: "Скажите «Гудвин» или напишите сообщение.",
    write: "Напишите сообщение ниже.", draft: "Написать Гудвину", send: "Отправить", you: "Вы",
    ready: "Ждёт слова «Гудвин»", paused: "Прослушивание на паузе", offline: "Микрофон недоступен",
    noModel: "Слово активации недоступно", disconnected: "Восстанавливаем связь…", connecting: "Подключаемся…",
    account: "Введите пароль от панели.", password: "Пароль", login: "Войти", logout: "Выйти",
    setupAccount: "Придумайте пароль для веб-панели.", confirmPassword: "Повторите пароль", createPassword: "Создать пароль",
    preferences: "Общие настройки", language: "Язык", listening: "Ждать слова «Гудвин»",
    systemPrompt: "Системный промпт",
    systemPromptHint: "Опишите роль, характер и манеру общения Гудвина в чате и на колонке. Пустое поле возвращает стандартную инструкцию.",
    systemPromptExample: "Ты Гудвин, спокойный домашний помощник с сухим чувством юмора. Обращайся на «ты» и отвечай по делу.",
    voiceCheck: "Проверка колонки", testHint: "Говорите рядом с колонкой. Она воспроизведёт ответ.",
    manual: "Запустить кнопкой", byWake: "Запустить словом «Гудвин»", last: "Последний запрос",
    noRequest: "Здесь появятся результат и длительность этапов.", completed: "Завершён", ms: "мс",
    provider: "OpenRouter", key: "Ключ API", keySaved: "Ключ настроен.", keyEmpty: "Добавьте ключ, чтобы отправлять запросы.",
    replaceKey: "Введите новый ключ", removeKey: "Удалить сохранённый ключ", save: "Сохранить", saved: "Сохранено",
    transcription: "Модель распознавания речи", chatModel: "Модель ответа", speech: "Модель озвучивания",
    voice: "Голос", sensitivity: "Чувствительность к слову «Гудвин»", advanced: "Модели и чувствительность",
    plugins: {
      title: "Плагины", empty: "Установленных плагинов пока нет.", upload: "Загрузить пакет",
      archive: "Выберите архив .tar.gz для установки или обновления.", remove: "Удалить", cancel: "Отмена",
      confirm: "Удалить плагин и все его сохранённые данные?", password: "Включите вход с паролем, чтобы управлять пакетами плагинов.",
      missing: "Эта страница плагина недоступна.", loading: "Загрузка плагина…", loadError: "Не удалось загрузить страницу плагина.",
      status: { starting: "Запускается", ready: "Готов", updating: "Обновляется", removing: "Удаляется", error: "Недоступен" },
    },
    stages: {
      awaiting_wake: "Скажите «Гудвин» рядом с колонкой", recording: "Слушаю", transcribing: "Распознаю речь",
      thinking: "Готовлю ответ", synthesizing: "Готовлю озвучивание", playing: "Воспроизвожу ответ",
    },
    errors: {
      busy: "Гудвин уже отвечает на запрос.", microphone: "Микрофон колонки недоступен.",
      model: "Модель слова активации недоступна.", missing_key: "Добавьте ключ OpenRouter в настройках.",
      cloud: "OpenRouter не смог обработать запрос.", playback: "Колонка не смогла воспроизвести ответ.",
      no_speech: "Не удалось расслышать запрос.", no_wake: "Слово активации не услышано.",
      wrong_password: "Проверьте пароль.", login_retry: "Подождите немного и попробуйте снова.",
      invalid_password: "Пароль должен быть от 8 до 256 байт.", password_mismatch: "Пароли не совпадают.",
      setup_complete: "Пароль уже задан. Войдите, чтобы продолжить.", password_save_failed: "Не удалось сохранить пароль. Попробуйте ещё раз.",
      save_failed: "Не удалось сохранить настройки.", invalid_settings: "Проверьте модели, голос и ключ API.",
      invalid_system_prompt: "Введите системный промпт длиной до 4 000 символов.",
      invalid_message: "Введите сообщение длиной до 2 000 символов.",
      connection: "Нет связи с Гудвином. Попробуйте после подключения.", internal: "Что-то пошло не так.",
      invalid_plugin_package: "Архив повреждён или несовместим с этим устройством.", plugin_archive_too_large: "Архив превышает 16 МиБ.",
      plugin_password_required: "Включите вход с паролем, чтобы управлять пакетами плагинов.", plugin_not_found: "Плагин не найден.",
      plugin_start_failed: "Плагин не запустился.", plugin_update_rolled_back: "Обновление не удалось. Предыдущая версия и её данные восстановлены.",
      plugin_storage_failed: "Гудвин не смог сохранить или восстановить файлы плагина.", plugin_unavailable: "Плагин недоступен.",
    },
  },
};

export type Language = keyof typeof words;
export type Words = typeof words.en;
export type Stage = keyof Words["stages"];

export function initialLanguage(): Language {
  const language = localStorage.getItem("goodwin-language");
  if (language === "ru" || language === "en") return language;
  return navigator.language.toLowerCase().startsWith("ru") ? "ru" : "en";
}
export function errorLabel(t: Words, code: string): string {
  return t.errors[code as keyof Words["errors"]] ?? t.errors.internal;
}


