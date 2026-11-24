# Legacy OpenRouter provider

Current Station releases provide OpenRouter through the [OpenAI-compatible WASM package](../openai-compatible/). Install that package and configure the OpenRouter connection there. On first installation it imports the standalone package's API key and retains existing assistant stage assignments.

The standalone package remains in the source tree for older installations and native development. To build and package it for a Windows development instance, run these commands from the repository root.

```powershell
cargo build --locked --release -p goodwin-openrouter
python scripts/package-plugin.py plugins/openrouter target/release/goodwin-openrouter.exe windows-x86_64 target/plugins/openrouter-windows-x86_64.tar.gz
```
