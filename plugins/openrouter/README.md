# OpenRouter provider

Upload `openrouter-linux-aarch64.tar.gz` in **Plugins**. Open **OpenRouter → Settings** to enter your API key. Open **Settings → Assistant** to select OpenRouter and a model for each stage that uses it. For speech synthesis, choose a voice or enter its ID.

When upgrading from the built-in provider, enter your key in the plugin's settings. Goodwin retains your provider assignments. Models previously set in an installed OpenRouter plugin still serve as defaults when no explicit model is selected.

The Windows Station build produces `target/plugins/openrouter-linux-aarch64.tar.gz`. To package the plugin for a Windows development instance:

```powershell
cargo build --locked --release -p goodwin-openrouter
python scripts/package-plugin.py plugins/openrouter target/release/goodwin-openrouter.exe windows-x86_64 target/plugins/openrouter-windows-x86_64.tar.gz
```
