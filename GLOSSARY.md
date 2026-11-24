# Goodwin

Goodwin is a voice assistant for Yandex Station Mini 3 (`bergamot`).

## Terms

**Control panel** — The browser interface for chatting with Goodwin, configuring the assistant and microphone, managing plugins, viewing diagnostics, and updating a managed Station.

**Assistant engine** — The mode Goodwin uses to handle an assistant request. Goodwin currently uses the cascade.

**Cascade (STT → LLM → TTS)** — The engine that transcribes the user's speech (STT), generates an answer with a language model (LLM), and turns the answer into audio (TTS). Text chat uses the LLM stage. Each stage has its own provider, model, and optional parameters, such as the synthesis voice and speaking rate.

**Model provider** — A plugin that connects Goodwin to an inference service and offers one or more model capabilities for the engine's stages. The OpenAI-compatible plugin can provide multiple named connections, including OpenRouter. The control panel can show a model catalog or accept a model ID manually.

**Provider connection** — A named API configuration with a stable provider ID, protocol, base URL, and optional key. Choose a connection and a model independently for each cascade stage.

**Plugin** — An extension that runs in a native process or as a WebAssembly (WASM) module in Wasmi. It can supply model capabilities, assistant tools, HTTP endpoints, or control panel pages.

**Plugin package** — An independently installable bundle with a manifest, a native executable or WASM module, and optional prebuilt control panel assets.

**Declarative plugin settings** — Fields a plugin defines in its package manifest. Goodwin renders the form and stores values such as service URLs and API keys per plugin. Open the form from the Plugins list or the header of a custom plugin page.

**Assistant tool** — A named operation that a plugin registers with Goodwin and the language model can choose to invoke when handling a request.

**Current conversation** — The messages shown in the web chat. It includes spoken and written exchanges and lasts until a new chat starts or Goodwin restarts.

**Wake word** — The word or phrase a user says to activate Goodwin. The selected model detects it locally.

**Wake-word model** — A local model that recognizes the activation phrase on the Station. Goodwin supports LiveKit Standard and Micro ONNX models. Station builds also support Micro INT8 TensorFlow Lite models.

**Wake word gain** — A microphone signal multiplier applied to the input of the wake-word model. Wake word sensitivity sets the threshold for accepting a detection.

**Recognition gain** — The input level adjustment for speech detection and recording sent to the STT stage. Goodwin can adjust this level automatically or apply a fixed multiplier.

**Voice request** — A question or command spoken to Goodwin. A regular voice request follows the wake word.

**Test request** — A voice request used to check Goodwin from the control panel. It starts with the wake word or the panel's recording button.

**Spoken response** — Goodwin's answer as heard through the Station's speaker.

**Speech speed** — The speaking-rate multiplier sent to the selected TTS model. A model that supports the setting treats 1× as its normal speed.

**Streaming playback** — Playing a spoken response as the TTS provider delivers chunks of PCM audio. Goodwin uses it by default when the plugin advertises streaming.

**Buffered playback** — Waiting for the TTS provider to return a complete audio response before playing it. Goodwin uses it when streaming is turned off or unavailable.

**Voice interaction** — An exchange that starts with the wake word or the panel's recording button. Each interaction handles one voice request.

**Station indicators** — The clock display and light ring that show the state of a voice interaction. Goodwin shows blinking eyes while recording, rotates the ring while processing, and animates the lower display segments with the audio level while speaking. The clock returns after the interaction.

**Station installer** — A desktop and command-line tool that installs or updates Goodwin on a prepared Station over ADB. Headless mode uses `goodwin-installer install` and prints the panel URL when it succeeds.
