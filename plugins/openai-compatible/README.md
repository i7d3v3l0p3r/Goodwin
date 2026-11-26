# OpenAI-compatible provider

The WASM package connects Goodwin to OpenRouter and to servers that implement OpenAI-style chat, transcription or speech synthesis. You can configure multiple named connections in one installation and choose a connection and model independently for each assistant stage.

1. Upload `openai-compatible-wasm32.tar.gz` in **Plugins** on a Station build with WASM support. The Windows Station build writes the archive to `target/plugins/`.
2. Open **OpenAI-compatible provider → Settings**. Enter a key in the OpenRouter connection or add an OpenAI-compatible connection with a name, base URL including the API prefix, and an optional key. Use the server's address as seen from the Station, for example `http://192.168.1.10:8000/v1`.
3. Select the connection by name for each stage in **Settings → Assistant**. Choose a model from its catalog or enter the model ID manually. Set a voice for speech servers that require one.
4. Save the assistant settings.

For KittenTTS 2, use the [official server](https://github.com/KittenML/KittenTTS/blob/main/docs/docker.md) and select the plugin for **Speech synthesis**. Enter the model ID `kitten-tts-2-latest` manually and use the `Russian` voice for Russian speech. Use the model and voice IDs accepted by your server if you run a different KittenTTS distribution.

For OpenAI-compatible connections, the plugin sends multipart WAV to `audio/transcriptions`, JSON to `chat/completions`, and requests raw PCM from `audio/speech`. Speech output defaults to 24 kHz mono unless the server includes `rate` and `channels` in its PCM media type. Use a chat model with tool calling support to control Goodwin plugins by voice.

For streaming voice playback on an OpenAI-compatible connection, the plugin sends `Accept: audio/pcm` and forwards complete PCM frames as the HTTP body arrives. OpenRouter speech uses buffered playback. The WASM package uses Goodwin's shared HTTP client.

The speech endpoints must return signed 16-bit little-endian PCM. Goodwin accepts one or two channels and sample rates from 8 to 96 kHz. With an unspecified rate and channel count it uses mono at 24 kHz. The plugin rejects conflicting rate or channel declarations, and rejects explicit metadata that describes a different sample encoding.

Goodwin reads model catalogs incrementally. The provider response may contain up to 8,000,000 bytes, and each model entry may contain up to 32 KiB. The plugin retains at most 750 compatible models within its reply size limit.

The native package configures a single OpenAI-compatible server through declarative plugin settings. To package that variant for a Windows development instance, build the panel first, then run these commands from the repository root.

```powershell
cargo build --locked --release -p goodwin-openai-compatible
python scripts/package-plugin.py plugins/openai-compatible target/release/goodwin-openai-compatible.exe windows-x86_64 target/plugins/openai-compatible-windows-x86_64.tar.gz
```
