# Wake-word dataset

Upload `wake-dataset-wasm32.tar.gz` on a Station build with WASM support. Updating the native package to WASM retains the recordings and settings. Sign in to the panel with a password and open **Wake-word dataset**. Start collection, then listen to the captured recordings and label each as **Wake word heard** or **No wake word**. You can change or clear a label or delete a recording.

The plugin starts with collection off. It saves a clip when a new frame reaches the configurable minimum score, which defaults to 0.1 and applies to collection only. Overlapping audio is kept as context. A qualifying score in the overlap alone does not save a second clip. Set the minimum to 0 to collect every window. The detector scores audio while listening for the wake word, and muting the microphone pauses it. A normal window lasts 2.56 seconds with a 1.28-second overlap. A wake detection or interruption can end a window early.

The plugin stops at 1,024 recordings, about 81 MiB of audio at most. **Delete all** stops collection, discards the current window, and removes all saved recordings and labels after confirmation. Plugin removal also deletes its recordings.

**Download labelled dataset** saves a TAR archive with `positive/` and `negative/` folders and a `manifest.json` containing each file's label, capture time, model ID and SHA-256, wake phrase, gain, threshold, and ordered frame scores. Each WAV is mono, 16 kHz, signed 16-bit PCM. New recordings contain the same gain-adjusted samples as the wake detector and have `info.gainApplied` set to `true`. Earlier recordings contain raw PCM and export with `info.gainApplied` set to `false`. The scores correspond to consecutive 80 ms frames evaluated with the recorded gain and the detector's streaming context. Only manually labelled files are exported. The archive leaves the recordings on the Station.

Group recordings by wake phrase for training and keep overlapping recordings together when splitting training and validation data. Training workflows that require phrase alignment need timestamps for positive examples.
