# OpenAI-compatible provider

The plugin connects Goodwin to an HTTP(S) server that implements OpenAI-style chat completions, transcription or speech synthesis. Configure one server address per installation of this plugin. In Goodwin Settings, choose the plugin for each stage that should use it. The other stages can use OpenRouter or other provider plugins.

1. Upload `openai-compatible-linux-aarch64.tar.gz` in **Plugins**. The Windows Station build writes it to `target/plugins/`.
2. Find **OpenAI-compatible provider** in **Plugins** and open its **Settings**. Enter the server's base URL including its API prefix, for example `http://192.168.1.10:8000/v1`. If the server runs on another device, use its address on the network. Enter the server's API key if it requires one.
3. Save, then select **OpenAI-compatible provider** for the corresponding stages in **Settings → Assistant**. Choose each stage's model from the server's catalog or enter the ID manually. Set a voice for speech servers that require one.
4. Save the assistant settings.

Re-enter your settings once if you are updating from the experimental package with its own settings page.

For KittenTTS 2, use the [official server](https://github.com/KittenML/KittenTTS/blob/main/docs/docker.md) and select the plugin for **Speech synthesis**. Enter the model ID `kitten-tts-2-latest` manually and use the `Russian` voice for Russian speech. Use the model and voice IDs accepted by your server if you run a different KittenTTS distribution.

The plugin sends multipart WAV to `audio/transcriptions`, JSON to `chat/completions`, and requests raw PCM from `audio/speech`. Speech output defaults to 24 kHz mono unless the server includes `rate` and `channels` in its PCM media type. Use a chat model with tool calling support to control Goodwin plugins by voice.

For voice playback the plugin sends `Accept: audio/pcm` and forwards complete PCM frames as the HTTP body arrives. A server that delivers audio while generating lets Goodwin start playback early. Servers that return the whole PCM body are supported with the same settings.

To package the plugin for a Windows development instance, build the panel first, then run these commands from the repository root.

```powershell
cargo build --locked --release -p goodwin-openai-compatible
python scripts/package-plugin.py plugins/openai-compatible target/release/goodwin-openai-compatible.exe windows-x86_64 target/plugins/openai-compatible-windows-x86_64.tar.gz
```
