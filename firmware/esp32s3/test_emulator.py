"""Exercise the Goodwin panel and push-to-talk in ESP-EMU with a loopback provider."""

import argparse
import base64
import csv
import hashlib
from email import policy
from email.parser import BytesParser
from http.server import BaseHTTPRequestHandler, HTTPServer
import io
import json
import math
import os
import gzip
from http.cookiejar import CookieJar
from pathlib import Path
from queue import Queue
import shutil
import socket
import struct
import subprocess
import sys
import tarfile
from threading import Event, Thread
from time import monotonic, sleep
from urllib.error import HTTPError
from urllib.request import HTTPCookieProcessor, Request, build_opener
import wave


PORT = 8765
BASE_URL = f"http://192.168.4.1:{PORT}/v1"
NVS_OFFSET = 0x9000
NVS_SIZE = 0x6000
PASSWORD_TIMEOUT = 150
TRANSCRIPT = "Какая погода?"
WAKE_TRANSCRIPT = "Гудвин, " + TRANSCRIPT
ANSWER = "Сегодня солнечно."
CHAT = "Добрый день!"
SPEECH_RATE = 24000
SPEECH_HZ = 660
CHAT_PREFIX = ANSWER.split()[0]
SYSTEM_PROMPT = "Goodwin ESP fixture system prompt."


def tone(rate=SPEECH_RATE, hz=SPEECH_HZ):
    return b"".join(
        struct.pack("<h", int(7000 * math.sin(math.tau * hz * i / rate)))
        for i in range(rate // 2)
    )


def check_tone(samples, rate, expected_hz):
    assert max(samples) > 1000, "Audio was silent or too quiet"
    # Discard startup/shutdown silence and measure only the steady tone.
    start = next(i for i, sample in enumerate(samples) if abs(sample) > 1000)
    segment = samples[start : start + rate // 5]
    assert len(segment) == rate // 5, "Audio ended before 200 ms of tone"
    crossings = sum(a <= 0 < b for a, b in zip(segment, segment[1:]))
    assert abs(crossings * 5 - expected_hz) <= 15, (crossings, expected_hz)


def tone_amplitude(samples, rate, frequency):
    phase = [i * math.tau * frequency / rate for i in range(len(samples))]
    sine = sum(sample * math.sin(angle) for sample, angle in zip(samples, phase))
    cosine = sum(sample * math.cos(angle) for sample, angle in zip(samples, phase))
    return 2 * math.hypot(sine, cosine) / len(samples)


def check_amplifier(samples, rate, timers):
    window = rate // 20
    longest_speech = longest_alarm = speech = alarm = 0
    for offset in range(0, len(samples) - window + 1, window):
        segment = samples[offset : offset + window]
        main = tone_amplitude(segment, rate, SPEECH_HZ)
        secondary = tone_amplitude(segment, rate, 880)
        speech = speech + 1 if main > 5000 and secondary < 1000 else 0
        alarm = alarm + 1 if main > 3000 and secondary > 1000 else 0
        longest_speech = max(longest_speech, speech)
        longest_alarm = max(longest_alarm, alarm)
    assert longest_speech >= 4, "The assistant did not produce 200 ms of the 660 Hz fixture tone"
    if timers:
        assert longest_alarm >= 4, "The timer alarm did not produce both tones for at least 200 ms"


WAKE_TAG = "v0.1.0"
WAKE_ID = "fixture-int8"
WAKE_CATALOG_LOCATION = f"/wake/download/{WAKE_TAG}/metadata.json?signature={'x' * 1024}"


def wake_catalog(path):
    # Trailing padding changes the digest without changing the FlatBuffer model.
    model = path.read_bytes() + bytes(16)
    selected = {
        "id": WAKE_ID, "wake_word": "Гудвин", "tier": "micro",
        "runtime": "livekit-v2-micro-tflite", "score_threshold": 0.390625,
        "sample_rate_hz": 16000, "format": "tflite", "filename": f"{WAKE_ID}.tflite",
        "size_bytes": len(model), "sha256": hashlib.sha256(model).hexdigest(),
    }
    catalog = {
        "schema_version": 1, "tag": WAKE_TAG,
        "models": [
            selected,
            dict(selected, id="fixture-onnx", filename="fixture-onnx.onnx",
                 runtime="livekit-v2-micro-onnx", format="onnx"),
            dict(selected, id="fixture-large", filename="fixture-large.tflite", size_bytes=600 * 1024),
        ],
    }
    return json.dumps(catalog, ensure_ascii=False).encode(), model


class Provider(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def setup(self):
        self.request.settimeout(10)
        super().setup()

    def log_message(self, _format, *args):
        pass

    def do_GET(self):
        try:
            assert self.headers.get("Authorization") is None
            kind = "application/json"
            if self.path == "/v1/models":
                self.server.requests.append(self.path)
                body = b'{"data":[{"id":"fixture-chat"},{"id":"fixture-stt"},{"id":"fixture-tts"}]}'
            elif self.server.wake_catalog and self.path == "/wake/latest/download/metadata.json":
                self.send_response(302)
                self.send_header("Location", WAKE_CATALOG_LOCATION)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            elif self.server.wake_catalog and self.path == WAKE_CATALOG_LOCATION:
                body = self.server.wake_catalog[0]
            else:
                assert self.server.wake_catalog and self.path == f"/wake/download/{WAKE_TAG}/{WAKE_ID}.tflite"
                kind = "application/octet-stream"
                body = self.server.wake_catalog[1]
                self.server.wake_downloads += 1
                if self.server.wake_downloads == 1:
                    body = body[:-1] + b"\x01"
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)
        except Exception as error:
            self.server.failure = error
            self.close_connection = True

    def do_POST(self):
        try:
            status, content_type, body = self.reply()
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Connection", "close")
            if content_type in ("audio/pcm", "text/event-stream"):
                self.send_header("Transfer-Encoding", "chunked")
                self.end_headers()
            if content_type == "text/event-stream":
                for index, chunk in enumerate(body):
                    self.wfile.write(f"{len(chunk):X}\r\n".encode() + chunk + b"\r\n")
                    self.wfile.flush()
                    if index == 0:
                        assert self.server.release_chat.wait(15), "Chat did not publish the partial reply"
                self.wfile.write(b"0\r\n\r\n")
            elif content_type == "audio/pcm":
                chunk_size = 3031  # Split some 16-bit samples across chunk boundaries.
                for index in range(0, len(body), chunk_size):
                    chunk = body[index : index + chunk_size]
                    self.wfile.write(f"{len(chunk):X}\r\n".encode() + chunk + b"\r\n")
                    # Queue three complete 4 KiB host reads before pausing the fixture.
                    if index == 4 * chunk_size and getattr(self.server, "streaming", False):
                        self.wfile.flush()
                        assert self.server.release_stream.wait(8), "Playback did not start before the full response"
                self.wfile.write(b"0\r\n\r\n")
            else:
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
        except Exception as error:
            self.server.failure = error
            self.close_connection = True

    def reply(self):
        assert self.headers.get("Authorization") is None
        size = int(self.headers["Content-Length"])
        assert 0 < size < 600000
        request = self.rfile.read(size)
        self.server.requests.append(self.path)
        if self.path == "/v1/audio/transcriptions":
            mime = self.headers["Content-Type"].encode()
            form = BytesParser(policy=policy.default).parsebytes(
                b"Content-Type: " + mime + b"\r\n\r\n" + request
            )
            fields = {part.get_param("name", header="content-disposition"): part
                      for part in form.iter_parts()}
            model, audio = fields["model"], fields["file"]
            assert model.get_payload(decode=True) == b"fixture-stt"
            if getattr(self.server, "plugin_provider", False):
                assert fields["language"].get_payload(decode=True) == b"ru"
            with wave.open(io.BytesIO(audio.get_payload(decode=True))) as wav:
                assert (wav.getnchannels(), wav.getsampwidth(), wav.getframerate()) == (1, 2, 16000)
                frames = wav.getnframes()
                samples = struct.unpack(f"<{frames}h", wav.readframes(frames))
                if getattr(self.server, "wake", False):
                    assert 3200 <= frames <= 16 * 16000, frames
                    assert max(samples) > 1000, "Wake recording lacks speech"
                else:
                    assert 3200 <= frames <= 32000, frames
                    check_tone(samples, 16000, 440)
            text = WAKE_TRANSCRIPT if getattr(self.server, "wake", False) else TRANSCRIPT
            return 200, "application/json", json.dumps({"text": text}).encode()
        if self.path == "/v1/chat/completions":
            chat = json.loads(request)
            assert chat["model"] == "fixture-chat"
            if getattr(self.server, "panel_settings", False):
                assert chat["messages"][0]["role"] == "system"
                assert chat["messages"][0]["content"].startswith(SYSTEM_PROMPT)
            last = chat["messages"][-1]
            if getattr(self.server, "tools", False) and last.get("role") == "tool":
                assert last["tool_call_id"] == "timer-call"
                assert json.loads(last["content"])["created"]["name"] == "Voice fixture"
            elif getattr(self.server, "tools", False) and last["content"] == TRANSCRIPT:
                assert any(tool["function"]["name"] == "timers__create" for tool in chat["tools"])
                assert chat["tool_choice"] == "auto"
                message = {"role": "assistant", "content": None, "tool_calls": [{
                    "id": "timer-call", "type": "function",
                    "function": {"name": "timers__create", "arguments": '{"seconds":900,"name":"Voice fixture"}'},
                }]}
                return 200, "application/json", json.dumps({"choices": [{"message": message}]}).encode()
            else:
                assert last in [
                    {"role": "user", "content": TRANSCRIPT}, {"role": "user", "content": CHAT}
                ]
            if chat.get("stream"):
                assert last == {"role": "user", "content": CHAT}
                events = [
                    {"choices": [{"delta": {"content": part}}]}
                    for part in (CHAT_PREFIX, ANSWER[len(CHAT_PREFIX):])
                ]
                chunks = [f"data: {json.dumps(event, ensure_ascii=False)}\n\n".encode() for event in events]
                return 200, "text/event-stream", [*chunks, b"data: [DONE]\n\n"]
            response = {"choices": [{"message": {"content": ANSWER}}]}
            return 200, "application/json", json.dumps(response, ensure_ascii=False).encode()
        assert self.path == "/v1/audio/speech"
        assert json.loads(request) == {
            "model": "fixture-tts", "input": ANSWER, "voice": "fixture", "response_format": "pcm"
        }
        return 200, "audio/pcm", tone()


def prepare_board(examples, work):
    modules = {
        "mic": ("audio/inmp441.wasm", {"SD": 7}),
        "amp": ("audio/max98357a.wasm", {"DIN": 6}),
        "button": ("button/pushbutton.wasm", {"A": 4}),
    }
    parts = {}
    for name, (source, pins) in modules.items():
        module = Path(source).name
        shutil.copyfile(examples / source, work / module)
        parts[name] = {"module": module, "pins": pins}
    board = work / "board.json"
    board.write_text(json.dumps({"parts": parts}), encoding="utf-8")
    return board


def prepare_flash(firmware, work, **provisioned):
    settings = {
        "ssid": "myssid",
        "wifi_password": "mypassword",
        **provisioned,
    }
    config = work / "fixture.csv"
    with config.open("w", newline="", encoding="utf-8") as file:
        writer = csv.writer(file)
        writer.writerow(("key", "type", "encoding", "value"))
        writer.writerow(("goodwin", "namespace", "", ""))
        writer.writerows((key, "data", "string", value) for key, value in settings.items())
    nvs = work / "fixture-nvs.bin"
    generator = (
        Path(os.environ["IDF_PATH"])
        / "components/nvs_flash/nvs_partition_generator/nvs_partition_gen.py"
    )
    subprocess.run([sys.executable, generator, "generate", config, nvs, str(NVS_SIZE)], check=True)
    image = bytearray(firmware.read_bytes())
    assert image[NVS_OFFSET : NVS_OFFSET + NVS_SIZE] == b"\xff" * NVS_SIZE
    image[NVS_OFFSET : NVS_OFFSET + NVS_SIZE] = nvs.read_bytes()
    flash = work / "fixture-flash.bin"
    flash.write_bytes(image)
    return flash


def control(port, *commands):
    with socket.create_connection(("127.0.0.1", port), timeout=5) as connection:
        with connection.makefile("rwb", buffering=0) as stream:
            for command in commands:
                stream.write(f"{command}\n".encode())
                reply = stream.readline().decode().strip()
                assert reply == "ok", reply


class Panel:
    def __init__(self, port):
        self.base = f"http://127.0.0.1:{port}"
        self.client = build_opener(HTTPCookieProcessor(CookieJar()))

    def request(self, route, method="GET", body=None, status=200, extra=None, timeout=20):
        headers = {"Accept": "application/json", **(extra or {})}
        if isinstance(body, bytes):
            headers["Content-Type"] = "application/octet-stream"
        elif body is not None:
            headers["Content-Type"] = "application/json"
            body = json.dumps(body).encode()
        req = Request(self.base + route,
                      data=body,
                      headers=headers, method=method)
        try:
            reply = self.client.open(req, timeout=timeout)
        except HTTPError as error:
            reply = error
        with reply:
            assert reply.code == status, (route, reply.code, reply.read())
            data = reply.read()
            if reply.headers.get("Content-Encoding") == "gzip":
                return gzip.decompress(data)
            if not data:
                return None
            return json.loads(data) if reply.headers.get_content_type() == "application/json" else data

    def install_timers(self, path):
        assert all(plugin["id"] != "timers" for plugin in self.request("/api/plugins"))
        archive = path.read_bytes()
        self.request("/api/plugin-packages", "POST", archive, 204, timeout=90)
        plugin = next(plugin for plugin in self.request("/api/plugins") if plugin["id"] == "timers")
        assert plugin["id"] == "timers" and plugin["status"] == "ready"
        assert plugin["pages"][0]["id"] == "main"
        assert b"<html" in self.request("/plugins/timers")
        asset = f"/plugin-assets/timers/{plugin['revision']}/{plugin['web']['entry']}"
        assert self.request(asset)
        assert self.request("/api/plugins/timers/timers")["timers"] == []
        self.print_resources("with Timers installed")

    def exercise_timers(self, path, voice_tool):
        route = "/api/plugins/timers/timers"
        if voice_tool:
            assert any(timer["name"] == "Voice fixture" for timer in self.request(route)["timers"])
        signal = self.request(route, "POST", {"seconds": 2, "name": "Sound check"}, 201)
        deadline = monotonic() + 25
        while monotonic() < deadline:
            timers = self.request(route)["timers"]
            if any(timer["id"] == signal["id"] and timer["completed"] for timer in timers):
                break
            sleep(0.4)
        else:
            raise AssertionError("The WASM timer did not finish")
        saved = self.request(route, "POST", {"seconds": 900, "name": "Persistent timer"}, 201)

        # Keep the package structure valid and force a guest startup failure on update.
        broken = io.BytesIO()
        with tarfile.open(path) as source, tarfile.open(
            fileobj=broken, mode="w:gz", format=tarfile.USTAR_FORMAT
        ) as target:
            for item in source:
                if item.name.endswith(".wasm"):
                    payload = b"invalid wasm"
                    item.size = len(payload)
                    target.addfile(item, io.BytesIO(payload))
                else:
                    target.addfile(item, source.extractfile(item))
        assert self.request("/api/plugin-packages", "POST", broken.getvalue(), 409, timeout=90) == {
            "error": "plugin_update_rolled_back",
        }
        self.request("/api/plugin-packages", "POST", path.read_bytes(), 204, timeout=90)
        assert saved in self.request(route)["timers"]
        self.print_resources("after Timers activation and replacement")

    def verify_timers_after_restart(self):
        route = "/api/plugins/timers/timers"
        assert any(plugin["id"] == "timers" and plugin["status"] == "ready"
                   for plugin in self.request("/api/plugins"))
        timers = self.request(route)["timers"]
        timer = next(timer for timer in timers if timer["name"] == "Persistent timer")
        assert not timer["completed"]
        self.request(f"{route}/{timer['id']}", "DELETE", status=204)
        self.request("/api/plugin-packages/timers", "DELETE", status=204)
        assert all(plugin["id"] != "timers" for plugin in self.request("/api/plugins"))
        assert self.request(route, status=404) == {"error": "plugin_not_found"}
        print("Verified WASM installation, assets, timer, failed and successful updates, restart and removal")

    def start_dataset(self, path):
        self.request("/api/plugin-packages", "POST", path.read_bytes(), 204, timeout=90)
        route = "/api/plugins/wake-dataset"
        assert not self.request(f"{route}/clips")["clips"]
        self.request(f"{route}/capture-settings", "PUT", {"minimumScore": 0.15}, 204)
        assert self.request(f"{route}/capture", "PUT", {"enabled": True})["collecting"]
        self.print_resources("with Wake-word dataset installed")

    def stop_dataset(self, expected_model):
        route = "/api/plugins/wake-dataset"
        status = self.request(f"{route}/capture", "PUT", {"enabled": False})
        assert not status["collecting"] and not status["storageFailed"]
        self.verify_dataset(expected_model)

    def verify_dataset(self, expected_model):
        route = "/api/plugins/wake-dataset"
        page = self.request(f"{route}/clips")
        assert page["clips"], page
        clip = max(page["clips"], key=lambda row: row["peak"])
        assert clip["info"]["model"]["id"] == expected_model and clip["peak"] >= 0.15
        assert clip["info"]["gainApplied"] is True
        audio = self.request(f"{route}/clips/{clip['id']}/audio")["wav"]
        with wave.open(io.BytesIO(base64.b64decode(audio))) as wav:
            assert (wav.getnchannels(), wav.getsampwidth(), wav.getframerate()) == (1, 2, 16000)
            assert wav.getnframes() == 1280 * len(clip["info"]["scores"])
            assert any(wav.readframes(wav.getnframes())), "Captured wake audio was silent"
        print("Verified Wake-word dataset metadata and recorded PCM")

    def install_wake_model(self):
        path = "/api/wake-models"
        catalog = self.request(path)
        assert catalog["tag"] == WAKE_TAG
        assert [model["id"] for model in catalog["models"]] == [WAKE_ID], catalog
        current = self.request("/api/state")["wake_model"]
        assert current["id"] == "goodwin-micro-int8"
        payload = {"id": WAKE_ID, "tag": WAKE_TAG}
        assert self.request(path, "PUT", dict(payload, tag="v0.0.1"), 409) == {
            "error": "wake_catalog_changed",
        }
        assert self.request(path, "PUT", payload, 502, timeout=100) == {"error": "wake_model_failed"}
        assert self.request("/api/state")["wake_model"] == current
        installed = self.request(path, "PUT", payload, timeout=100)
        assert installed == catalog["models"][0]
        assert self.request("/api/state")["wake_model"] == installed
        self.print_resources("after changing the wake model")
        return installed

    def configure_plugin_provider(self, path, streaming):
        self.request("/api/plugin-packages", "POST", path.read_bytes(), 204, timeout=90)
        connection_path = "/api/plugin-connections/openai-compatible"
        assert self.request(connection_path)[0]["providerId"] == "openai-compatible:openrouter"
        result = self.request(connection_path, "PUT", {
            "name": "ESP fixture", "protocol": "openai", "baseUrl": BASE_URL, "key": "",
        })
        connection = next(row for row in result["connections"] if row["id"] == result["id"])
        assert not connection["hasKey"] and "key" not in connection
        provider = connection["providerId"]
        models = self.request(f"/api/plugin-models/{provider.replace(':', '%3A')}/chat", timeout=60)
        assert models["available"] and any(model["id"] == "fixture-chat" for model in models["models"])
        pipeline = {
            stage: {"provider": provider, "selection": {"model": model}}
            for stage, model in [
                ("transcription", "fixture-stt"), ("chat", "fixture-chat"), ("speech", "fixture-tts"),
            ]
        }
        pipeline["speech"]["selection"]["parameters"] = {"voice": "fixture"}
        pipeline["speech_streaming"] = streaming
        self.request("/api/settings/assistant", "PUT", {"engine": "pipeline", "pipeline": pipeline}, 204)
        self.verify_plugin_provider()
        self.print_resources("with OpenAI-compatible WASM provider installed")

    def verify_plugin_provider(self):
        connection = next(row for row in self.request("/api/plugin-connections/openai-compatible")
                          if row["name"] == "ESP fixture")
        state = self.request("/api/state")
        assert state["provider_ready"] and state["voice_provider_ready"]
        pipeline = state["settings"]["assistant"]["pipeline"]
        assert all(pipeline[stage]["provider"] == connection["providerId"]
                   for stage in ("transcription", "chat", "speech"))

    def setup_access(self):
        assert self.request("/api/access") == {
            "protected": True, "authorized": False, "setup_required": True
        }
        assert self.request("/api/state", status=401) == {"error": "login_required"}
        assert self.request("/api/diagnostics/logs", status=401) == {"error": "login_required"}
        assert b"<div id=\"app\"" in self.request("/settings/assistant")
        assert b"push_to_talk" in self.request("/panel.js")
        assert len(self.request("/plugin-runtime-v1.js")) > 1000
        assert len(self.request("/panel.css")) > 1000
        assert self.request("/api/capabilities")["provider_configuration"] == "builtin"
        password = "goodwin-fixture"
        setup = {"password": password, "confirmation": password}
        assert self.request("/api/setup", "POST", setup, 403,
                            {"Origin": "https://example.invalid"}) == {"error": "origin"}
        self.request("/api/setup", "POST", setup, 204, timeout=PASSWORD_TIMEOUT)
        assert self.request("/api/access")["authorized"]

    def wait_network(self, expected):
        deadline = monotonic() + 15
        while True:
            state = self.request("/api/network")
            if state["status"] == expected:
                return state
            assert monotonic() < deadline, (expected, state)
            sleep(0.1)

    def configure_and_chat(self, streaming, provider_package=None, release_chat=None):
        wifi = self.wait_network("connected")
        assert wifi["ssid"] == "myssid" and "password" not in wifi
        state = self.request("/api/state")
        assert not state["provider_ready"]
        audio = {**state["settings"], "listening": False, "sensitivity": 0.55,
                 "wake_gain": 1.0, "transcription_gain": 1.0}
        self.request("/api/settings", "PUT", {"settings": {**audio, "wake_gain": 0}}, 400)
        self.request("/api/settings", "PUT", {"settings": audio}, 204)
        self.request("/api/settings/microphone", "PUT", {"muted": True}, 204)
        assert self.request("/api/test", "POST", {"mode": "manual"}, 409) == {
            "error": "microphone_muted"
        }
        self.request("/api/settings/microphone", "PUT", {"muted": False}, 204)
        assert self.request("/api/chat", "POST", {"message": CHAT}, 409) == {
            "error": "provider_unavailable"
        }
        provider = {
            "base_url": "https://example.invalid/v1",
            "api_key": "example-secret",
            "stt_model": "fixture-stt",
            "chat_model": "fixture-chat",
            "tts_model": "fixture-tts",
            "voice": "fixture",
            "speech_streaming": streaming,
        }
        view = self.request("/api/settings/provider", "PUT", provider)
        assert view["has_key"] and "api_key" not in view
        provider["base_url"] = BASE_URL
        del provider["api_key"]
        view = self.request("/api/settings/provider", "PUT", provider)
        assert view == {**provider, "has_key": False}
        if provider_package:
            self.configure_plugin_provider(provider_package, streaming)
        assert self.request("/api/state")["provider_ready"]
        self.request("/api/chat", "POST", {"message": CHAT}, 202)
        deadline = monotonic() + 20
        while True:
            state = self.request("/api/state")
            if state["active"] and state["messages"][-1]["text"] == CHAT_PREFIX and release_chat is not None:
                release_chat.set()
            if state["active"] is None:
                break
            assert monotonic() < deadline, "Text chat did not complete"
            sleep(0.1)
        assert [(item["role"], item["text"]) for item in state["messages"]] == [
            ("user", CHAT), ("assistant", ANSWER)
        ]
        if provider_package and streaming:
            assert release_chat.is_set(), "Did not observe the intermediate plugin chat reply"
        self.print_resources("idle")

    def start_wake(self):
        state = self.request("/api/state")
        assert state["model"] and state["wake_model"]["runtime"] == "livekit-v2-micro-tflite"
        self.request("/api/settings", "PUT", {"settings": {**state["settings"], "listening": True}}, 204)

    def start_wake_test(self):
        assert not self.request("/api/state")["settings"]["listening"]
        self.request("/api/test", "POST", {"mode": "wake"}, 202)
        active = self.request("/api/state")["active"]
        assert active["source"] == "wake_test" and active["stage"] == "awaiting_wake"

    def set_system_prompt(self):
        assert self.request("/api/settings/system-prompt", "PUT", {"system_prompt": "a" * 4001}, 400) == {
            "error": "invalid_system_prompt",
        }
        self.request("/api/settings/system-prompt", "PUT", {"system_prompt": f" {SYSTEM_PROMPT} "}, 204)
        self.verify_panel_settings()

    def verify_panel_settings(self):
        assert self.request("/api/state")["system_prompt"] == SYSTEM_PROMPT
        logs = self.request("/api/diagnostics/logs")
        assert logs["supported"] and logs["kernel"] is None
        assert "Ready for push-to-talk" in logs["goodwin"] and "password=" not in logs["goodwin"]

    def verify_voice(self, source, streaming):
        state = self.request("/api/state")
        report = state["last_voice"]
        assert report["source"] == source
        assert report["started_at_ms"] == 0
        assert report["error"] is None
        assert report["transcript"] == (WAKE_TRANSCRIPT if source in ("wake", "wake_test") else TRANSCRIPT)
        assert report["answer"] == ANSWER
        assert state["messages"][-2]["text"] == TRANSCRIPT
        assert report["playback"] == ("streaming" if streaming else "buffered")
        stages = [
            "recording", "transcribing", "thinking", "synthesizing", "playing"
        ]
        if source == "wake_test":
            stages.insert(0, "awaiting_wake")
            assert not state["settings"]["listening"]
        assert [timing["stage"] for timing in report["timings"]] == stages
        resources = self.print_resources("after voice")
        assert resources["psram"]["total_bytes"] == 8 * 1024 * 1024
        assert resources["memory"]["used_bytes"] > 0
        self.request("/api/chat/clear", "POST", {}, 204)
        assert self.request("/api/state")["messages"] == []

    def print_resources(self, stage):
        resources = self.request("/api/diagnostics/resources")
        for region in ("memory", "psram"):
            heap = resources[region]
            assert 0 <= heap["minimum_free_bytes"] <= heap["total_bytes"] - heap["used_bytes"]
        print(f"ESP diagnostics {stage}", json.dumps(resources), flush=True)
        return resources

    def install_firmware(self, image):
        assert self.request("/api/firmware", status=401) == {"error": "login_required"}
        self.request("/api/login", "POST", {"password": "goodwin-fixture"}, 204, timeout=PASSWORD_TIMEOUT)
        assert self.request("/api/firmware")["running_slot"] == "ota_0"
        assert self.request("/api/firmware", "POST", bytes(4096), 400) == {"error": "invalid_firmware"}
        assert self.request("/api/firmware")["running_slot"] == "ota_0"
        self.request("/api/firmware", "POST", image.read_bytes(), 202, timeout=90)
        self.print_resources("before OTA reboot")

    def verify_after_restart(self, wake, streaming):
        assert self.request("/api/access") == {
            "protected": True, "authorized": False, "setup_required": False
        }
        self.request("/api/login", "POST", {"password": "goodwin-fixture"}, 204, timeout=PASSWORD_TIMEOUT)
        assert self.wait_network("connected")["ssid"] == "myssid"
        assert self.request("/api/settings/provider") == {
            "base_url": BASE_URL, "has_key": False, "stt_model": "fixture-stt",
            "chat_model": "fixture-chat", "tts_model": "fixture-tts", "voice": "fixture",
            "speech_streaming": streaming,
        }
        audio = self.request("/api/state")["settings"]
        assert audio["listening"] == wake and not audio["microphone_muted"]
        assert audio["sensitivity"] == 0.55 and audio["wake_gain"] == audio["transcription_gain"] == 1.0


def run_emulator(args, flash, board, output, release_stream, release_chat=None):
    with flash.open("rb") as flash_file:
        flash_file.seek(0x8000)
        table = flash_file.read(0xC00)
    ota_1 = next(
        address for magic, kind, subtype, address, *_ in struct.iter_unpack("<HBBII16sI", table)
        if (magic, kind, subtype) == (0x50AA, 0, 0x11)
    )
    with socket.socket() as control_socket:
        control_socket.bind(("127.0.0.1", 0))
        port = control_socket.getsockname()[1]
    with socket.socket() as http:
        http.bind(("127.0.0.1", 0))
        panel_port = http.getsockname()[1]
    panel = Panel(panel_port)
    active_model = "goodwin-micro-int8"
    selection = None
    config = args.work / "esp-emu.json"
    config.write_text('{"autoupdate":false}', encoding="utf-8")
    command = [
        args.emulator, "--chip", "esp32s3", "--firmware", flash, "--psram-size", "8M",
        "--cosim-board", board, "--part-wav", f"amp={output}",
        "--control-tcp", f"127.0.0.1:{port}", "--net",
        f"user,net=192.168.4.0/24,hostfwd=tcp:127.0.0.1:{panel_port}-:80",
        "--config", config, "--log-color", "never", "--timeout",
        "420s" if args.timers_package or args.dataset_package or args.provider_package else ("300s" if args.update_image else "220s"),
        "--wifi-fw-ap-password", "goodwin-setup",
    ]
    if args.wake_audio:
        command.extend(["--part-wav-in", f"mic={args.wake_audio}"])
    elif args.manual_test:
        recording = args.work / "manual-input.wav"
        with wave.open(str(recording), "wb") as wav:
            wav.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
            wav.writeframes(tone(16000, 440) + bytes(2 * 16000 * 2))
        command.extend(["--part-wav-in", f"mic={recording}"])
    capabilities = None
    psram_ready = False
    started = False
    played = False
    persisted = False
    interrupted = False
    provision_steps = 0
    with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          text=True, encoding="utf-8", errors="replace") as process:
        lines = Queue()

        def read_uart():
            for line in process.stdout:
                print(line, end="", flush=True)
                lines.put(line)
            lines.put(None)

        Thread(target=read_uart, daemon=True).start()
        try:
            deadline = monotonic() + (
                445 if args.timers_package or args.dataset_package or args.provider_package
                else (325 if args.update_image else 245)
            )
            while (line := lines.get(timeout=max(0.1, deadline - monotonic()))) is not None:
                if "Capabilities " in line:
                    capabilities = json.loads(line.split("Capabilities ", 1)[1])
                psram_ready |= "PSRAM 8388608 bytes" in line
                if args.interrupt_update and played and not interrupted and f"Loaded app from partition at offset {ota_1:#x}" in line:
                    control(port, "reset")
                    interrupted = True
                if args.provision_wifi and "DHCP server assigned IP to a client" in line:
                    if provision_steps == 0:
                        panel.request("/api/network", status=401)
                        panel.setup_access()
                        setup = panel.wait_network("setup")
                        assert setup["ssid"].startswith("Goodwin-") and "password" not in setup
                        assert setup["error"] is None
                        invalid = {"ssid": "", "password": "too-short"}
                        assert panel.request("/api/network", "PUT", invalid, 400) == {"error": "invalid_wifi"}
                        panel.request("/api/network", "PUT", {"ssid": "myssid", "password": "wrong-pass"}, 202)
                    elif provision_steps == 1:
                        setup = panel.wait_network("setup")
                        assert setup["error"] == "wifi_connect_failed"
                        panel.request("/api/network", "PUT", {"ssid": "myssid", "password": "mypassword"}, 202)
                    else:
                        raise RuntimeError("Wi-Fi setup reopened after provisioning")
                    provision_steps += 1
                if "Goodwin panel http://" in line:
                    if not started:
                        if not args.provision_wifi:
                            if args.wake_catalog_model:
                                panel.request("/api/wake-models", status=401)
                            panel.setup_access()
                        if args.panel_settings:
                            panel.set_system_prompt()
                        panel.configure_and_chat(args.stream_speech, args.provider_package, release_chat)
                        if args.timers_package:
                            panel.install_timers(args.timers_package)
                        if args.wake_catalog_model:
                            selection = panel.install_wake_model()
                            active_model = selection["id"]
                        if args.dataset_package:
                            panel.start_dataset(args.dataset_package)
                        if args.wake_test:
                            panel.start_wake_test()
                        elif args.wake_audio:
                            panel.start_wake()
                        elif args.manual_test:
                            panel.request("/api/test", "POST", {"mode": "manual"}, 202)
                        else:
                            control(port, "part button input press after 100", "part button input release after 1200")
                        started = True
                    elif played and not persisted:
                        panel.verify_after_restart(bool(args.wake_audio and not args.wake_test), args.stream_speech)
                        if args.panel_settings:
                            panel.verify_panel_settings()
                        if args.timers_package:
                            panel.verify_timers_after_restart()
                        if args.dataset_package:
                            panel.verify_dataset(active_model)
                        if selection is not None:
                            assert panel.request("/api/state")["wake_model"] == selection
                            print("Verified catalog filtering, rejected checksum mismatch and restored the installed wake model")
                        if args.provider_package:
                            panel.verify_plugin_provider()
                        if args.update_image:
                            firmware = panel.request("/api/firmware")
                            if args.interrupt_update:
                                assert interrupted and firmware["running_slot"] == "ota_0"
                            else:
                                assert firmware["running_slot"] == "ota_1" and firmware["verified"]
                        persisted = True
                        process.terminate()
                if "Voice turn failed:" in line:
                    raise RuntimeError(line.strip())
                if args.stream_speech and "Streamed playback started" in line:
                    assert panel.request("/api/state")["active"]["stage"] == "playing"
                    panel.print_resources("during streaming")
                    release_stream.set()
                if "Voice turn complete" in line and not played:
                    source = ("wake_test" if args.wake_test else "wake" if args.wake_audio
                              else "manual_test" if args.manual_test else "push_to_talk")
                    panel.verify_voice(source, args.stream_speech)
                    if args.dataset_package:
                        panel.stop_dataset(active_model)
                    if args.timers_package:
                        panel.exercise_timers(args.timers_package, args.timer_voice_tool)
                    panel.request("/api/logout", "POST", {}, 204)
                    assert panel.request("/api/settings/provider", status=401) == {"error": "login_required"}
                    played = True
                    if args.update_image:
                        panel.install_firmware(args.update_image)
                    else:
                        control(port, "reset")
            if process.wait(timeout=5) != 0:
                raise RuntimeError(f"ESP-EMU exited with code {process.returncode}")
        finally:
            if process.poll() is None:
                process.kill()
    expected = {
        "platform": "espidf", "device": "esp32s3-generic", "activation": ["push_to_talk"],
        "indicators": [], "wake_runtimes": [], "speech_streaming": True,
        "panel_updates": "poll", "provider_configuration": "builtin",
        "plugin_packages": not args.no_plugins,
        "wake_catalog": bool(args.wake_build or args.wake_audio or args.wake_catalog_model),
    }
    if expected["wake_catalog"]:
        expected["activation"].append("wake_word")
        expected["wake_runtimes"].append("livekit-v2-micro-tflite")
    assert capabilities == expected, capabilities
    assert psram_ready, "The ESP32-S3 did not report 8 MiB of PSRAM"
    assert started and played, "Firmware did not finish a voice turn"
    assert persisted, "Panel configuration was not verified after restart"
    if args.provision_wifi:
        assert provision_steps == 2, "Wi-Fi provisioning and retry were not exercised"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--emulator", default="esp-emu")
    parser.add_argument("--examples", type=Path, required=True)
    parser.add_argument("--firmware", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--wake-build", action="store_true", help="Expect a wake-enabled firmware image")
    parser.add_argument("--no-plugins", action="store_true", help="Expect a firmware built without WASM")
    parser.add_argument("--timers-package", type=Path, help="Exercise the real Timers WASM package")
    parser.add_argument("--timer-voice-tool", action="store_true", help="Let the fixture invoke Timers during the voice turn")
    parser.add_argument("--dataset-package", type=Path, help="Exercise the real Wake-word dataset WASM package")
    parser.add_argument("--provider-package", type=Path, help="Use the real OpenAI-compatible WASM package")
    activation = parser.add_mutually_exclusive_group()
    activation.add_argument("--wake-audio", type=Path, help="Use a 16 kHz mono PCM WAV containing the wake word")
    activation.add_argument("--manual-test", action="store_true", help="Start the microphone check from the panel")
    parser.add_argument("--provision-wifi", action="store_true", help="Start with blank NVS and verify Wi-Fi setup")
    parser.add_argument("--update-image", type=Path, help="Verify an OTA install and reboot using this app.bin")
    parser.add_argument("--interrupt-update", action="store_true", help="Reset before OTA verification and check rollback")
    parser.add_argument("--stream-speech", action="store_true", help="Verify playback starts before the whole audio response arrives")
    parser.add_argument("--panel-settings", action="store_true", help="Check the saved system prompt and protected diagnostic log")
    parser.add_argument("--wake-test", action="store_true", help="Run the wake-word check with continuous listening paused")
    parser.add_argument("--wake-catalog-model", type=Path, help="Serve and install a Micro INT8 fixture model from a test catalog")
    args = parser.parse_args()
    if args.interrupt_update and not args.update_image:
        parser.error("--interrupt-update requires --update-image")
    if (args.timers_package or args.dataset_package or args.provider_package) and args.no_plugins:
        parser.error("Plugin packages require plugin support")
    if args.dataset_package and not args.wake_audio:
        parser.error("--dataset-package requires --wake-audio")
    if args.wake_test and not args.wake_audio:
        parser.error("--wake-test requires --wake-audio")
    if args.timer_voice_tool and not args.timers_package:
        parser.error("--timer-voice-tool requires --timers-package")
    args.work.mkdir(parents=True, exist_ok=True)
    board = prepare_board(args.examples, args.work)
    if args.provision_wifi:
        flash = args.work / "fixture-flash.bin"
        shutil.copyfile(args.firmware, flash)
    else:
        flash = prepare_flash(args.firmware, args.work)
    output = args.work / "reply.wav"
    output.unlink(missing_ok=True)
    with HTTPServer(("127.0.0.1", PORT), Provider) as server:
        server.requests = []
        server.failure = None
        server.wake = bool(args.wake_audio)
        server.streaming = args.stream_speech
        server.tools = args.timer_voice_tool
        server.plugin_provider = bool(args.provider_package)
        server.panel_settings = args.panel_settings
        server.wake_catalog = wake_catalog(args.wake_catalog_model) if args.wake_catalog_model else None
        server.wake_downloads = 0
        server.release_stream = Event()
        server.release_chat = Event()
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            run_emulator(args, flash, board, output, server.release_stream, server.release_chat)
        finally:
            server.shutdown()
            thread.join()
            if server.failure:
                raise server.failure
        expected = [
            "/v1/chat/completions", "/v1/audio/transcriptions",
            "/v1/chat/completions", "/v1/audio/speech"
        ]
        if args.timer_voice_tool:
            expected.insert(-1, "/v1/chat/completions")
        if args.provider_package:
            expected.insert(0, "/v1/models")
        assert server.requests == expected, server.requests
        assert server.wake_downloads == (2 if args.wake_catalog_model else 0)
    with wave.open(str(output)) as wav:
        assert (wav.getnchannels(), wav.getsampwidth(), wav.getframerate()) == (1, 2, 16000)
        frames = wav.getnframes()
        samples = struct.unpack(f"<{frames}h", wav.readframes(frames))
        check_amplifier(samples, 16000, args.timers_package)
    print(f"Verified panel setup, chat, diagnostics and the 660 Hz amplifier response in {output}")


if __name__ == "__main__":
    main()
