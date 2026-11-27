# Spotify playback with Soloist

This native Goodwin plugin searches Spotify for a track, album or playlist and sends its URI to a signed-in Soloist receiver. Soloist streams the music to its own audio output. Goodwin needs a voice assistant with tool calling enabled.

## Set up the receiver

1. Get your own Soloist API key and the official build for the playback device. Setting up Soloist requires Spotify Premium. Download Soloist separately and update it when its build expires. [Soloist setup](https://developer.spotify.com/documentation/soloist/tutorials/getting-started) · [Downloads](https://developer.spotify.com/documentation/soloist/reference/downloads-and-updates)
2. On Linux or macOS, start the receiver with the following command after setting the `SOLOIST_API_KEY` environment variable. On Windows PowerShell use `./soloist.exe` and `$env:SOLOIST_API_KEY` in the same command.

   ```sh
   ./soloist --device-name Goodwin --api-key "$SOLOIST_API_KEY" --ws 127.0.0.1:9090
   ```

3. In the Spotify app on the same network, choose **Goodwin** in the device picker and play a track. Check that you hear it on the intended speaker. Leave Soloist running. Its local WebSocket has no authentication. For a receiver on another computer, forward its loopback port over a private tunnel to the Goodwin host. [Receiver API](https://developer.spotify.com/documentation/soloist/reference/websocket-api)

The current plugin connects to an existing receiver. On Linux, Soloist uses PipeWire or PulseAudio. Goodwin on Mini 3 normally uses ALSA through `aplay -D speakers`. One existing bergamot setup runs Soloist with a private AArch64 loader, matching libraries and an apulse adapter for ALSA. The official Soloist 1.4.0.21 ARM64 binary passed a `--version` check under QEMU with that runtime. Playback on the physical Mini 3 with this build remains unverified. [Soloist requirements](https://github.com/spotify/soloist#requirements)

For a receiver on a separate computer, you can pair the Mini 3 to that computer as a Bluetooth speaker and select it as the computer's audio output. [Station Bluetooth audio](https://alice.yandex.ru/support/ru/station/speaker)

Goodwin on ESP32-S3 runs WASM add-ons on ESP-IDF. A controller for that platform would need an HTTP bridge to Soloist or a WebSocket host service. Audio would still play on the receiver. Playing it on the ESP32-S3 speaker would also require a network audio input for the I²S output. [ESP32-S3 audio and plugin support](../../firmware/esp32s3/README.md)

## Configure Goodwin

1. Create a Spotify Web API app for catalogue access and obtain its client ID and client secret. The plugin uses the Client Credentials flow and renews the app token in memory. The Soloist API key belongs on the receiver. Spotify's public Developer Policy §III.3 restricts voice control. Check the terms applicable to your integration before use. [Client Credentials](https://developer.spotify.com/documentation/web-api/tutorials/client-credentials-flow) · [Developer Policy](https://developer.spotify.com/policy)
2. Build the native package with `./scripts/build-station.ps1`. Upload `target/plugins/spotify-linux-aarch64.tar.gz` under **Plugins** in a password-protected Goodwin panel.
3. Open **Spotify → Settings**, enter the Web API client ID and secret, and set the two-letter country code of the listening account. The Soloist address defaults to `ws://127.0.0.1:9090` on the Goodwin host. Change it when using another forwarded port or a private WebSocket proxy.

Try “Гудвин, включи в Spotify песню «Группа крови» группы «Кино»”, “поставь Spotify на паузу”, “продолжи музыку” or “следующий трек”. A play request selects the first available match of the requested type in the configured market. A request for a playlist searches the public catalogue. [Search API](https://developer.spotify.com/documentation/web-api/reference/search)

Slow searches can continue after the voice reply. Ask whether the last Spotify request succeeded to read its status. A new playback command cancels any pending search. Soloist acknowledges dispatch before the receiver necessarily produces sound.

## Player in the Goodwin panel

With the VLESS VPN plugin installed, turn on **VPN** next to Spotify in **Plugins**. This routes Spotify account, search and cover-art requests. The plugin communicates with the Soloist receiver over its configured local connection. Soloist handles audio playback and its own network traffic.

Open **Spotify** in the sidebar or mobile menu. On an older panel, open **Plugins → Spotify**. The player page at `/plugins/spotify` shows the receiver's actual Spotify Connect name. For the first connection, play a track in the Spotify app on the same Wi-Fi and select that receiver in the device picker. Soloist saves the session in its data directory. The Web API credentials in plugin settings are for catalogue searches. [Soloist authentication](https://developer.spotify.com/documentation/soloist/concepts/authentication)

The page shows the current track, cover art when available, artists and playback position, with the transport controls offered by the receiver. **Queue** shows up to 20 upcoming items. **History** shows up to 20 previous items in the current Soloist queue, newest first. Playback on other devices and older account listening history are outside that queue. The page refreshes while visible and does not require catalogue credentials. [Soloist WebSocket API](https://developer.spotify.com/documentation/soloist/reference/websocket-api)
