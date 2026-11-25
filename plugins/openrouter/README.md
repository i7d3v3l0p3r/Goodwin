# Standalone OpenRouter provider

Current Station releases provide OpenRouter through the [OpenAI-compatible WASM package](../openai-compatible/). Configure the OpenRouter connection in that package's settings.

To build and package the standalone provider for a Windows development instance, run these commands from the repository root.

```powershell
cargo build --locked --release -p goodwin-openrouter
python scripts/package-plugin.py plugins/openrouter target/release/goodwin-openrouter.exe windows-x86_64 target/plugins/openrouter-windows-x86_64.tar.gz
```
