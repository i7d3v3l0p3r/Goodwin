# Goodwin

Goodwin is a voice assistant for Yandex Station.

## Terms

**Control panel** — The browser interface where the Station's owner configures Goodwin, checks its status, and runs a test request.

**Plugin** — A module that adds a capability to Goodwin and may supply assistant tools, HTTP endpoints, and pages or navigation entries for the control panel.

**Plugin package** — An independently installable bundle with a manifest, a native executable, and optional prebuilt control panel assets.

**Assistant tool** — A named operation that a plugin registers with Goodwin and the language model can choose to invoke when handling a request.

**Current conversation** — The messages shown in the web chat. It includes spoken and written exchanges and lasts until a new chat starts or Goodwin restarts.

**Wake word** — The word a user says to activate Goodwin. The first wake word is “Гудвин”.

**Voice request** — A question or command spoken to Goodwin. A regular voice request follows the wake word.

**Test request** — A voice request used to check Goodwin from the control panel. It starts with the wake word or the panel's recording button.

**Spoken response** — Goodwin's answer as heard through the Station's speaker.

**Voice interaction** — An exchange that starts with the wake word or the panel's recording button. The first version handles one voice request per interaction.
