"""Exercise the Goodwin panel and push-to-talk in ESP-EMU with a loopback provider."""

import argparse
import base64
from contextlib import closing
import csv
import hashlib
from email import policy
from email.parser import BytesParser
from http.client import HTTPConnection
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
from time import monotonic, sleep, time
from urllib.error import HTTPError
from urllib.request import HTTPCookieProcessor, Request, build_opener
import wave


PORT = 8765
BASE_URL = f"http://192.168.4.1:{PORT}/v1"
NVS_OFFSET = 0x9000
NVS_SIZE = 0x6000
PASSWORD_TIMEOUT = 210
TRANSCRIPT = "Какая погода?"
WAKE_TRANSCRIPT = "Гудвин, " + TRANSCRIPT
ANSWER = "Сегодня солнечно."
FOLLOW_UP = "А завтра?"
CHAT = "Добрый день!"
SPEECH_RATE = 24000
SPEECH_HZ = 660
CHAT_PREFIX = ANSWER.split()[0]
SYSTEM_PROMPT = "Goodwin ESP fixture system prompt."
LANGUAGES = {"transcription": "en", "reply": "de"}
PANEL_CONTRACT = Path(__file__).resolve().parents[2] / "tests/fixtures/panel_contract.json"


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


def check_amplifier(samples, rate, timers, interrupted=False, alarm_during_stream=False):
    window = rate // 20
    if interrupted:
        # The fixture pauses the stream and the emulator can insert silence
        # while it waits for TCP. Check the tone in the tail of the final reply.
        samples = samples[-rate:]
    longest_speech = longest_alarm = speech = alarm = 0
    audible_speech = 0
    alarm_after_speech = resumed_speech = False
    for offset in range(0, len(samples) - window + 1, window):
        segment = samples[offset : offset + window]
        main = tone_amplitude(segment, rate, SPEECH_HZ)
        secondary = tone_amplitude(segment, rate, 880)
        speech = speech + 1 if main > 5000 and secondary < 1000 else 0
        audible_speech += speech != 0
        alarm = alarm + 1 if main > 3000 and secondary > 1000 else 0
        longest_speech = max(longest_speech, speech)
        longest_alarm = max(longest_alarm, alarm)
        alarm_after_speech |= bool(longest_speech and alarm >= 4)
        resumed_speech |= bool(alarm_after_speech and speech)
    assert (audible_speech if interrupted else longest_speech) >= 4, "The assistant did not produce 200 ms of the 660 Hz fixture tone"
    if timers:
        assert longest_alarm >= 4, "The timer alarm did not produce both tones for at least 200 ms"
    if alarm_during_stream:
        assert resumed_speech, "The amplifier did not play speech before and after the timer alarm"


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
                if self.server.wake_catalog_requests == 0:
                    self.server.catalog_request_started.set()
                    assert self.server.release_catalog.wait(15), "Panel did not respond during catalog download"
                self.server.wake_catalog_requests += 1
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
            headers_sent = False
            if self.path == "/v1/audio/transcriptions" and (hold := self.server.hold_transcription):
                if hold == "release":
                    # Split headers after Content-Type to exercise a retry on an
                    # already active parser, including status and type retention.
                    self.send_response(status)
                    self.send_header("Content-Type", content_type)
                    self.flush_headers()
                    headers_sent = True
                self.server.transcription_started.set()
                assert self.server.release_transcription.wait(12), "Transcription fixture was not released"
                if hold == "cancel":
                    self.close_connection = True
                    return
            if not headers_sent:
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
                        assert self.server.release_stream.wait(self.server.stream_timeout), "The paused speech stream was not released"
                        if self.server.cancel_stream:
                            self.close_connection = True
                            self.server.stream_cancelled.set()
                            return
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
            assert fields["language"].get_payload(decode=True).decode() == LANGUAGES["transcription"]
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
            if self.server.dialogue and self.server.requests.count(self.path) == 2:
                text = FOLLOW_UP
            return 200, "application/json", json.dumps({"text": text}).encode()
        if self.path == "/v1/chat/completions":
            chat = json.loads(request)
            assert chat["model"] == "fixture-chat"
            assert f'ISO 639-1 code "{LANGUAGES["reply"]}"' in chat["messages"][0]["content"]
            if getattr(self.server, "panel_settings", False):
                assert chat["messages"][0]["role"] == "system"
                assert chat["messages"][0]["content"].startswith(SYSTEM_PROMPT)
            last = chat["messages"][-1]
            if last.get("content") == FOLLOW_UP:
                history = chat["messages"][1:-1]
                if history and history[-1]["role"] == "user":
                    assert "pluginContext" in json.loads(history.pop()["content"])
                assert history == [
                    {"role": "user", "content": TRANSCRIPT},
                    {"role": "assistant", "content": ANSWER},
                ]
                assert last["role"] == "user"
            elif getattr(self.server, "tools", False) and last.get("role") == "tool":
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
        self.port = port
        self.base = f"http://127.0.0.1:{port}"
        self.cookies = CookieJar()
        self.client = build_opener(HTTPCookieProcessor(self.cookies))

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
            allowed = status if isinstance(status, tuple) else (status,)
            assert reply.code in allowed, (route, reply.code, reply.read())
            data = reply.read()
            if reply.headers.get("Content-Encoding") == "gzip":
                data = gzip.decompress(data)
            elif not data:
                data = None
            elif reply.headers.get_content_type() == "application/json":
                data = json.loads(data)
            return (reply.code, data) if isinstance(status, tuple) else data

    def open_upload(self, route, size, origin=None):
        request = Request(self.base + route)
        self.cookies.add_cookie_header(request)
        connection = HTTPConnection("127.0.0.1", self.port, timeout=5)
        connection.putrequest("POST", route)
        if cookie := request.get_header("Cookie"):
            connection.putheader("Cookie", cookie)
        if origin:
            connection.putheader("Origin", origin)
        connection.putheader("Content-Type", "application/octet-stream")
        connection.putheader("Content-Length", str(size))
        connection.endheaders()
        return connection

    def streaming_upload(self, route, path, expected, interrupted=False):
        size = path.stat().st_size
        assert size > 4096
        with closing(self.open_upload(route, size)) as upload, path.open("rb") as body:
            upload.send(body.read(4096))
            # A distinct connection must be served while the worker is waiting
            # for the body. The SDK needs the first body byte to dispatch.
            assert self.request("/api/access", timeout=5)["authorized"]
            assert "settings" in self.request("/api/state", timeout=5)
            with closing(self.open_upload(route, size)) as competing:
                competing.send(b"\0")
                reply = competing.getresponse()
                assert (reply.status, json.load(reply)) == (409, {"error": "busy"})
            if interrupted:
                upload.sock.settimeout(30)
                upload.sock.shutdown(socket.SHUT_WR)
            else:
                upload.sock.settimeout(90)
                upload.send(body)
            reply = upload.getresponse()
            content = reply.read()
            assert reply.status == expected, (route, reply.status, content)
            print(f"Verified panel responsiveness during {route} ({'interrupted' if interrupted else 'complete'})",
                  flush=True)
            return json.loads(content) if content else None

    def install_timers(self, path):
        assert all(plugin["id"] != "timers" for plugin in self.request("/api/plugins"))
        initial_memory = self.request("/api/diagnostics/resources")["memory"]["used_bytes"]
        assert self.streaming_upload("/api/plugin-packages", path, 400, interrupted=True) == {
            "error": "invalid_plugin_package",
        }
        assert all(plugin["id"] != "timers" for plugin in self.request("/api/plugins"))
        self.streaming_upload("/api/plugin-packages", path, 204)
        plugin = next(plugin for plugin in self.request("/api/plugins") if plugin["id"] == "timers")
        assert plugin["id"] == "timers" and plugin["status"] == "ready"
        assert plugin["pages"][0]["id"] == "main"
        assert b"<html" in self.request("/plugins/timers")
        asset = f"/plugin-assets/timers/{plugin['revision']}/{plugin['web']['entry']}"
        assert self.request(asset)
        assert self.request("/api/plugins/timers/timers")["timers"] == []
        resources = self.print_resources("with Timers installed")
        assert resources["memory"]["used_bytes"] - initial_memory < 16 * 1024, (
            "The finished upload still holds its task stack", initial_memory, resources["memory"],
        )

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
        # Timer completion is saved before audio starts. Package replacement can
        # briefly be busy while the audio output still owns the alarm.
        deadline = monotonic() + 8
        while True:
            failure = self.request("/api/plugin-packages", "POST", broken.getvalue(), 409, timeout=90)
            if failure == {"error": "plugin_update_rolled_back"}:
                break
            assert failure == {"error": "plugin_busy"} and monotonic() < deadline, failure
            sleep(0.4)
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

    def install_dataset(self, path, capture):
        self.request("/api/plugin-packages", "POST", path.read_bytes(), 204, timeout=90)
        route = "/api/plugins/wake-dataset"
        page = self.request(f"{route}/clips")
        assert not page["clips"]
        assert page["status"]["storageMiB"] == 4
        assert page["status"]["maxStorageMiB"] == 2
        assert page["status"]["storageLimitBytes"] == 2 * 1024 * 1024
        self.request(f"{route}/capture-settings", "PUT", {"storageMiB": 3}, 400)
        self.request(f"{route}/capture-settings", "PUT", {"minimumScore": 0.15}, 204)
        if capture:
            status = self.request(f"{route}/capture", "PUT", {"enabled": True})
        else:
            status = self.request(f"{route}/clips")["status"]
        assert status["collecting"] == capture and status["storageMiB"] == 4
        assert 2 * 1024 * 1024 - 8192 < status["storageLimitBytes"] < 2 * 1024 * 1024
        self.request(f"{route}/capture-settings", "PUT", {"storageMiB": 1}, 204)
        status = self.request(f"{route}/clips")["status"]
        assert status["collecting"] == capture and status["storageMiB"] == 1
        assert status["storageLimitBytes"] == 1024 * 1024
        print("Verified Wake-word dataset storage quota and saved a lower limit")
        self.print_resources("with Wake-word dataset installed")

    def stop_dataset(self, expected_model):
        route = "/api/plugins/wake-dataset"
        status = self.request(f"{route}/capture", "PUT", {"enabled": False})
        assert not status["collecting"] and not status["storageFailed"]
        self.verify_dataset(expected_model)

    def verify_dataset(self, expected_model):
        route = "/api/plugins/wake-dataset"
        page = self.request(f"{route}/clips")
        assert page["status"]["storageMiB"] == 1
        assert page["status"]["maxStorageMiB"] == 2
        assert page["status"]["storageLimitBytes"] == 1024 * 1024
        if expected_model is None:
            assert not page["clips"] and not page["status"]["collecting"]
            print("Verified Wake-word dataset storage quota after restart")
            return
        assert page["clips"], page
        clip = max(page["clips"], key=lambda row: row["peak"])
        assert clip["info"]["model"]["id"] == expected_model and clip["peak"] >= 0.15
        assert clip["info"]["gainApplied"] is True
        audio = self.request(f"{route}/clips/{clip['id']}/audio")["wav"]
        with wave.open(io.BytesIO(base64.b64decode(audio))) as wav:
            assert (wav.getnchannels(), wav.getsampwidth(), wav.getframerate()) == (1, 2, 16000)
            frame_samples = clip["info"]["frameSamples"]
            assert frame_samples == (480 if expected_model == "goodwin-microwakeword" else 1280)
            assert wav.getnframes() == frame_samples * len(clip["info"]["scores"])
            assert any(wav.readframes(wav.getnframes())), "Captured wake audio was silent"
        print("Verified Wake-word dataset metadata and recorded PCM")

    def install_wake_model(self, catalog_sync):
        path = "/api/wake-models"
        result = path + "/result"
        ticket = self.request(path, status=202, timeout=5)

        def check_panel_while_fetching():
            started, release = catalog_sync
            assert started.wait(5), "The device did not start the catalog download"
            assert self.request("/api/access", timeout=5)["authorized"]
            assert self.request(path, status=409, timeout=5) == {"error": "wake_model_busy"}
            release.set()

        catalog = self.wait_operation(result, ticket, 200, 95, check_panel_while_fetching)
        assert catalog["tag"] == WAKE_TAG
        assert [model["id"] for model in catalog["models"]] == [WAKE_ID, "goodwin-microwakeword"], catalog
        current = self.request("/api/state")["wake_model"]
        builtin = catalog["models"][1]
        assert all(current[key] == builtin[key] for key in ("id", "runtime", "sha256")), (current, builtin)
        assert math.isclose(current["score_threshold"], builtin["score_threshold"], abs_tol=1e-7), (current, builtin)
        assert current["runtime"] == "microwakeword-tflite"
        payload = {"id": WAKE_ID, "tag": WAKE_TAG}
        ticket = self.request(path, "PUT", dict(payload, tag="v0.0.1"), 202, timeout=5)
        assert self.wait_operation(result, ticket, 409, 100) == {"error": "wake_catalog_changed"}
        ticket = self.request(path, "PUT", payload, 202, timeout=5)
        assert self.wait_operation(result, ticket, 502, 100) == {"error": "wake_model_failed"}
        assert self.request("/api/state")["wake_model"] == current
        ticket = self.request(path, "PUT", payload, 202, timeout=5)
        installed = self.wait_operation(result, ticket, 200, 100)
        assert installed == catalog["models"][0]
        assert self.request(result, "POST", ticket) == installed
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

    def verify_plugin_update(self, path):
        def provider():
            return next(item for item in self.request("/api/plugins") if item["id"] == "openai-compatible")

        def package(version, broken):
            output = io.BytesIO()
            with tarfile.open(path) as source, tarfile.open(fileobj=output, mode="w:gz") as target:
                for member in source:
                    contents = source.extractfile(member).read()
                    if member.name == "manifest.json":
                        manifest = json.loads(contents)
                        manifest["version"] = version
                        contents = json.dumps(manifest).encode()
                    if broken and member.name.endswith(".wasm"):
                        contents = b"\0asm\x01\0\0\0"
                    member.size = len(contents)
                    target.addfile(member, io.BytesIO(contents))
            return output.getvalue()

        previous = provider()
        version = "99.0.0"
        route = "/api/plugin-packages"
        assert self.request(route, "POST", package(version, True), 409, timeout=90) == {
            "error": "plugin_update_rolled_back",
        }
        assert provider()["revision"] == previous["revision"]
        self.verify_plugin_provider()
        self.request(route, "POST", package(version, False), 204, timeout=90)
        current = provider()
        assert current["version"] == version and current["revision"] != previous["revision"]
        self.verify_plugin_provider()
        print("Verified plugin update and rollback preserve saved provider connections")

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
        assert self.request("/api/access/open", "POST", {}, 403,
                            {"Origin": "https://example.invalid"}) == {"error": "origin"}
        self.request("/api/access/open", "POST", {}, 204)
        assert self.request("/api/access") == {
            "protected": False, "authorized": True, "setup_required": False,
        }
        assert self.request("/api/diagnostics/logs", status=403) == {"error": "diagnostics_password_required"}
        assert self.request("/api/firmware", status=403) == {"error": "core_update_password_required"}
        self.submit_password("/api/access/password", setup)
        assert self.request("/api/access")["authorized"]
        assert Panel(self.port).request("/api/access/open", "POST", {}, 401) == {"error": "login_required"}

    def submit_password(self, route, body):
        ticket = self.request(route, "POST", body, 202, timeout=5)
        access = self.request("/api/access", timeout=5)
        assert not access["protected"] or not access["authorized"]
        result = "/api/auth/result"
        assert self.request(result, "POST", ticket, 403, {
            "Origin": "https://example.invalid",
        }) == {"error": "origin"}
        assert self.request(result, "POST", {"operation": "unknown"}, 404) == {
            "error": "auth_result_expired",
        }
        def check_panel_while_hashing():
            access = self.request("/api/access", timeout=5)
            assert not access["protected"] or not access["authorized"]
            assert self.request(route, "POST", body, 429, timeout=5) == {"error": "login_retry"}

        self.wait_operation(result, ticket, 204, PASSWORD_TIMEOUT, check_panel_while_hashing)
        cookie = next(cookie for cookie in self.cookies if cookie.name == "goodwin_session")
        assert abs(cookie.expires - time() - 30 * 24 * 60 * 60) < 5
        assert self.request(result, "POST", ticket, 404) == {"error": "auth_result_expired"}

    def wait_operation(self, route, ticket, expected, timeout, while_pending=None):
        deadline = monotonic() + timeout
        while monotonic() < deadline:
            status, data = self.request(route, "POST", ticket, (202, expected), timeout=5)
            if status == expected:
                assert while_pending is None, f"{route} did not expose a pending state"
                return data
            assert data == {"pending": True}
            if while_pending is not None:
                while_pending()
                while_pending = None
            sleep(0.75)
        raise AssertionError(f"{route} did not finish")

    def wait_network(self, expected):
        deadline = monotonic() + 15
        while True:
            state = self.request("/api/network")
            if state["status"] == expected:
                return state
            assert monotonic() < deadline, (expected, state)
            sleep(0.1)

    def configure_and_chat(self, streaming, provider_package=None, release_chat=None, dialogue=False):
        wifi = self.wait_network("connected")
        assert wifi["ssid"] == "myssid" and "password" not in wifi
        state = self.request("/api/state")
        assert not state["provider_ready"]
        assert state["settings"]["languages"] == {"transcription": "ru", "reply": "ru"}
        assert state["settings"]["dialogue"] == {"mode": "follow_up", "timeout_seconds": 8}
        self.dialogue = {"mode": "follow_up" if dialogue else "single_turn", "timeout_seconds": 6}
        audio = {**state["settings"], "listening": False, "sensitivity": 0.55,
                 "wake_gain": 1.0, "transcription_gain": 1.0, "dialogue": self.dialogue,
                 "languages": LANGUAGES}
        for invalid in [0, 61]:
            self.request("/api/settings", "PUT", {"settings": {**audio, "dialogue": {
                **self.dialogue, "timeout_seconds": invalid}}}, 400)
        self.request("/api/settings", "PUT", {"settings": {**audio, "wake_gain": 0}}, 400)
        self.request("/api/settings", "PUT", {"settings": {**audio, "languages": {
            **LANGUAGES, "transcription": "xx"}}}, 400)
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
        assert state["model"] and state["wake_model"]["runtime"] in ("microwakeword-tflite", "livekit-v2-micro-tflite")
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
        metrics = self.request("/api/diagnostics/metrics")
        assert metrics["voice_cancellations"] == 0
        assert metrics["flash_writes"]["count"] > 0
        assert metrics["flash_writes"]["max_us"] >= metrics["flash_writes"]["p95_upper_us"] > 0

        def numeric(value):
            if isinstance(value, dict):
                return all(numeric(item) for item in value.values())
            return value is None or type(value) is int

        assert numeric(metrics), metrics

    def verify_panel_contract(self):
        profile = self.request("/api/capabilities")
        assert (profile["platform"], profile["panel_updates"]) == ("espidf", "poll")
        examples = json.loads(PANEL_CONTRACT.read_text(encoding="utf-8"))
        for example in examples:
            if example.get("requiresWakeCatalog") and not profile["wake_catalog"]:
                continue
            response = self.request(example["path"], example["method"], example["body"], example["status"])
            assert response == example["response"], (example, response)
        state = self.request("/api/state")
        assert state["system_prompt"] == "Contract fixture\nNext line."
        assert not state["settings"]["microphone_muted"]

    def verify_asset_cache(self):
        path = "/panel.js"
        policy = "public, max-age=0, must-revalidate"
        with closing(HTTPConnection("127.0.0.1", self.port, timeout=5)) as connection:
            connection.request("GET", path)
            with connection.getresponse() as reply:
                assert reply.status == 200
                assert reply.getheader("Content-Encoding") == "gzip"
                assert reply.getheader("Cache-Control") == policy
                body = reply.read()
                tag = reply.getheader("ETag")
                assert tag == f'"{hashlib.sha256(body).hexdigest()}"'
                assert gzip.decompress(body)

            connection.request("GET", path, headers={"If-None-Match": f'"stale", W/{tag}'})
            with connection.getresponse() as reply:
                assert reply.status == 304
                assert reply.getheader("ETag") == tag
                assert reply.getheader("Cache-Control") == policy
                assert not reply.read()

            connection.request("GET", path, headers={"If-None-Match": '"stale"'})
            with connection.getresponse() as reply:
                assert reply.status == 200 and reply.read() == body

            connection.request("GET", "/settings")
            with connection.getresponse() as reply:
                assert reply.status == 200
                assert reply.getheader("Content-Encoding") == "gzip"
                assert reply.getheader("Cache-Control") == "no-store"
                assert b"<html" in gzip.decompress(reply.read())

            connection.request("GET", "/api/capabilities")
            with connection.getresponse() as reply:
                assert reply.status == 200
                assert reply.getheader("Cache-Control") == "no-store"
                reply.read()
        print("Verified gzip, conditional asset caching, and uncacheable HTML and API")

    def mute_microphone_and_submit_stale_settings(self):
        original = self.request("/api/state")["settings"]
        self.request("/api/settings/microphone", "PUT", {"muted": True}, 204)
        self.request("/api/settings", "PUT", {"settings": original}, 204)
        assert self.request("/api/state")["settings"]["microphone_muted"]

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
        transcript = FOLLOW_UP if source == "follow_up" else TRANSCRIPT
        assert report["transcript"] == (WAKE_TRANSCRIPT if source in ("wake", "wake_test") else transcript)
        assert report["answer"] == ANSWER
        assert state["messages"][-2]["text"] == transcript
        assert report["playback"] == ("streaming" if streaming else "buffered")
        stages = [
            "recording", "transcribing", "thinking", "synthesizing", "playing"
        ]
        if source == "wake_test":
            stages.insert(0, "awaiting_wake")
            assert not state["settings"]["listening"]
        if source == "follow_up":
            stages.insert(0, "awaiting_speech")
        assert [timing["stage"] for timing in report["timings"]] == stages
        resources = self.print_resources("after voice")
        assert resources["psram"]["total_bytes"] == 8 * 1024 * 1024
        assert resources["memory"]["used_bytes"] > 0
        if source in ("wake", "wake_test"):
            metrics = self.request("/api/diagnostics/metrics")
            inference = metrics["wake_inference"]
            assert inference["count"] > 0
            assert 0 < inference["p50_upper_us"] <= inference["p95_upper_us"] <= inference["max_us"]
            frame_samples = 480 if state["wake_model"]["runtime"] == "microwakeword-tflite" else 1280
            assert metrics["wake_capture_samples"] >= inference["count"] * frame_samples
            assert 0 < metrics["wake_max_lag_samples"] <= metrics["wake_capture_samples"]
            print("ESP wake inference", json.dumps({
                **inference,
                "capture_skipped_frames": metrics["capture_skipped_frames"],
                "wake_capture_samples": metrics["wake_capture_samples"],
                "wake_max_lag_samples": metrics["wake_max_lag_samples"],
                "wake_inference_failed": metrics["wake_inference_failed"],
            }, sort_keys=True), flush=True)
        return report

    def wait_voice(self, predicate, timeout=12):
        end = monotonic() + timeout
        while monotonic() < end:
            state = self.request("/api/state")
            if result := predicate(state):
                return result
            sleep(0.04)
        raise AssertionError(("Voice request did not reach the expected state", state["active"], state["last_voice"]))

    def exercise_voice_lifecycle(self, port, server):
        def start():
            control(port, "part button input press after 100", "part button input release after 1200")
            return self.wait_voice(lambda state: state["active"])["request_id"]

        def cancel(request_id, stage):
            self.request("/api/voice/cancel", "POST", {"request_id": request_id}, 202)
            last = self.wait_voice(lambda state: not state["active"] and state["last_voice"])
            assert (last["request_id"], last["error"], last["error_stage"]) == (request_id, "cancelled", stage)

        start_requests = len(server.requests)
        if self.request("/api/state")["model"]:
            self.request("/api/test", "POST", {"mode": "wake"}, 202)
            wake = self.wait_voice(lambda state: state["active"])
            assert wake["stage"] == "awaiting_wake"
            cancel(wake["request_id"], "awaiting_wake")
        recording = start()
        cancel(recording, "recording")
        assert len(server.requests) == start_requests
        # Let the first button release before pressing again.
        sleep(2)

        server.hold_transcription = "cancel"
        processing = start()
        assert processing != recording
        assert server.transcription_started.wait(10)
        old = self.request("/api/voice/cancel", "POST", {"request_id": recording}, 409)
        assert old == {"error": "voice_request_inactive"}
        cancel(processing, "transcribing")
        server.release_transcription.set()

        server.transcription_started.clear()
        server.release_transcription.clear()
        server.hold_transcription = "release"
        server.cancel_stream = True
        playing = start()
        assert server.transcription_started.wait(10)
        connection_path = "/api/plugin-connections/openai-compatible"
        connection = next(row for row in self.request(connection_path) if row["name"] == "ESP fixture")
        self.request(f"{connection_path}/{connection['id']}", "DELETE")
        sleep(0.8)
        server.release_transcription.set()
        self.wait_voice(lambda state: state["active"] and state["active"]["stage"] == "playing")
        assert server.playback_started.wait(7)
        cancel(playing, "playing")
        server.release_stream.set()
        assert server.stream_cancelled.wait(5)
        server.release_stream.clear()
        assert server.requests[start_requests:] == [
            "/v1/audio/transcriptions", "/v1/audio/transcriptions",
            "/v1/chat/completions", "/v1/audio/speech",
        ]
        # Configure the next request with a new connection. The cancelled turn used
        # the old connection through TTS after it had been removed from the panel.
        original = self.request("/api/state")["settings"]["assistant"]
        result = self.request(connection_path, "PUT", {
            "name": "ESP fixture", "protocol": "openai", "baseUrl": BASE_URL, "key": "",
        })
        for capability in ("transcription", "chat", "speech"):
            original["pipeline"][capability]["provider"] = f"openai-compatible:{result['id']}"
        self.request("/api/settings/assistant", "PUT", original, 204)
        self.request("/api/chat/clear", "POST", {}, 204)
        server.hold_transcription = None
        server.cancel_stream = False
        print("Verified ESP recording, stalled WASM HTTP, and streamed playback cancellation with pinned providers", flush=True)

    def print_resources(self, stage):
        resources = self.request("/api/diagnostics/resources")
        for region in ("memory", "psram"):
            heap = resources[region]
            assert 0 <= heap["minimum_free_bytes"] <= heap["total_bytes"] - heap["used_bytes"]
        print(f"ESP diagnostics {stage}", json.dumps(resources), flush=True)
        return resources

    def install_firmware(self, image):
        assert self.request("/api/firmware", status=401) == {"error": "login_required"}
        with closing(self.open_upload("/api/firmware", 4096)) as connection:
            connection.send(b"\0")
            reply = connection.getresponse()
            assert (reply.status, json.load(reply)) == (401, {"error": "login_required"})
        self.submit_password("/api/login", {"password": "goodwin-fixture"})
        with closing(self.open_upload("/api/firmware", 4096, origin="https://example.invalid")) as connection:
            connection.send(b"\0")
            reply = connection.getresponse()
            assert (reply.status, json.load(reply)) == (403, {"error": "origin"})
        assert self.request("/api/firmware")["running_slot"] == "ota_0"
        assert self.request("/api/firmware", "POST", bytes(4096), 400) == {"error": "invalid_firmware"}
        assert self.request("/api/firmware")["running_slot"] == "ota_0"
        assert self.streaming_upload("/api/firmware", image, 500, interrupted=True) == {
            "error": "firmware_update_failed",
        }
        assert self.request("/api/firmware")["running_slot"] == "ota_0"
        assert self.streaming_upload("/api/firmware", image, 202) == {"restarting": True}
        self.print_resources("before OTA reboot")

    def verify_after_restart(self, wake, streaming, microphone_muted):
        assert self.request("/api/access") == {
            "protected": True, "authorized": False, "setup_required": False
        }
        self.submit_password("/api/login", {"password": "goodwin-fixture"})
        assert self.wait_network("connected")["ssid"] == "myssid"
        assert self.request("/api/settings/provider") == {
            "base_url": BASE_URL, "has_key": False, "stt_model": "fixture-stt",
            "chat_model": "fixture-chat", "tts_model": "fixture-tts", "voice": "fixture",
            "speech_streaming": streaming,
        }
        audio = self.request("/api/state")["settings"]
        assert audio["listening"] == wake and audio["microphone_muted"] == microphone_muted
        assert audio["sensitivity"] == 0.55 and audio["wake_gain"] == audio["transcription_gain"] == 1.0
        assert audio["dialogue"] == self.dialogue
        assert audio["languages"] == LANGUAGES


def run_emulator(args, flash, board, output, server):
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
    active_model = "goodwin-microwakeword"
    selection = None
    config = args.work / "esp-emu.json"
    config.write_text('{"autoupdate":false}', encoding="utf-8")
    has_plugins = args.timers_package or args.dataset_package or args.provider_package
    limit = 420 if has_plugins else 220
    if args.update_image:
        limit = 480 if has_plugins else 360
    if args.emulator_timeout:
        limit = args.emulator_timeout
    command = [
        args.emulator, "--chip", "esp32s3", "--firmware", flash, "--psram-size", "8M",
        "--cosim-board", board, "--part-wav", f"amp={output}",
        "--control-tcp", f"127.0.0.1:{port}", "--net",
        f"user,net=192.168.4.0/24,hostfwd=tcp:127.0.0.1:{panel_port}-:80",
        "--config", config, "--log-color", "never", "--timeout", f"{limit}s",
        "--wifi-fw-ap-password", "goodwin-setup",
    ]
    if args.wake_audio:
        command.extend(["--part-wav-in", f"mic={args.wake_audio}"])
    elif args.manual_test or args.dialogue:
        recording = args.work / "voice-input.wav"
        with wave.open(str(recording), "wb") as wav:
            wav.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
            pause_seconds = 5 if args.dialogue else 2
            wav.writeframes(tone(16000, 440) + bytes(pause_seconds * 16000 * 2))
            if args.dialogue:
                wav.writeframes(tone(16000, 440) + bytes(30 * 16000 * 2))
        command.extend(["--part-wav-in", f"mic={recording}"])
    capabilities = None
    psram_ready = False
    started = False
    played = False
    dialogue_started = False
    persisted = False
    interrupted = False
    provision_steps = 0
    playback_events = 0
    wifi_trial = False
    wifi_joining = False
    alarm_during_join = False
    stream_alarm_started = None

    def activate_voice():
        if args.wake_test:
            panel.start_wake_test()
        elif args.wake_audio:
            panel.start_wake()
        elif args.manual_test:
            panel.request("/api/test", "POST", {"mode": "manual"}, 202)
        else:
            control(port, "part button input press after 100", "part button input release after 1200")

    with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          text=True, encoding="utf-8", errors="replace") as process:
        lines = Queue()

        def read_uart():
            for line in process.stdout:
                print(line, end="", flush=True)
                if "Streamed playback started" in line:
                    server.playback_started.set()
                lines.put(line)
            lines.put(None)

        Thread(target=read_uart, daemon=True).start()
        try:
            deadline = monotonic() + limit + 25
            while (line := lines.get(timeout=max(0.1, deadline - monotonic()))) is not None:
                if "Capabilities " in line:
                    capabilities = json.loads(line.split("Capabilities ", 1)[1])
                psram_ready |= "PSRAM 8388608 bytes" in line
                if wifi_trial and "Applying Wi-Fi settings" in line:
                    wifi_joining = True
                if wifi_joining and "Played plugin timer alarm" in line:
                    alarm_during_join = True
                if stream_alarm_started is not None and "Played plugin timer alarm" in line:
                    alarm_time = int(line.split("(", 1)[1].split(")", 1)[0])
                    delay = alarm_time - stream_alarm_started
                    assert 0 < delay < 6000, f"The stream held the timer alarm for {delay} ms"
                    assert not server.release_stream.is_set()
                    assert panel.request("/api/state")["active"]["stage"] == "playing"
                    stream_alarm_started = None
                    server.release_stream.set()
                    print(f"Verified timer audio {delay} ms after streamed playback started, while the provider held the response open")
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
                            panel.verify_panel_contract()
                            panel.verify_asset_cache()
                            panel.set_system_prompt()
                        panel.configure_and_chat(args.stream_speech, args.provider_package, server.release_chat, args.dialogue)
                        if args.timers_package:
                            panel.install_timers(args.timers_package)
                        if args.wake_catalog_model:
                            selection = panel.install_wake_model((server.catalog_request_started, server.release_catalog))
                            active_model = selection["id"]
                        if args.dataset_package:
                            panel.install_dataset(args.dataset_package, capture=bool(args.wake_audio))
                        if args.voice_lifecycle:
                            panel.verify_plugin_update(args.provider_package)
                            panel.exercise_voice_lifecycle(port, server)
                        started = True
                        if args.wifi_during_alarm:
                            panel.request("/api/plugins/timers/timers", "POST",
                                          {"seconds": 2, "name": "Wi-Fi retry alarm"}, 201)
                            panel.request("/api/network", "PUT", {"ssid": "myssid", "password": "wrong-pass"}, 202)
                            wifi_trial = True
                        else:
                            activate_voice()
                    elif wifi_trial:
                        assert wifi_joining and alarm_during_join, "Wi-Fi reconfiguration blocked the timer audio"
                        wifi = panel.wait_network("connected")
                        assert wifi["ssid"] == "myssid" and wifi["error"] == "wifi_connect_failed", wifi
                        wifi_trial = wifi_joining = False
                        print("Verified timer audio during Wi-Fi reconfiguration and restored the previous network")
                        activate_voice()
                    elif played and not persisted:
                        panel.verify_after_restart(
                            bool(args.wake_audio and not args.wake_test), args.stream_speech, args.panel_settings
                        )
                        if args.panel_settings:
                            panel.verify_panel_settings()
                            print("Verified that the microphone stayed muted after reset")
                        if args.timers_package:
                            panel.verify_timers_after_restart()
                        if args.dataset_package:
                            panel.verify_dataset(active_model if args.wake_audio else None)
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
                    playback_events += 1
                    if args.voice_lifecycle and playback_events == 1:
                        continue
                    assert panel.request("/api/state")["active"]["stage"] == "playing"
                    panel.print_resources("during streaming")
                    if args.alarm_during_stream:
                        panel.request("/api/plugins/timers/timers", "POST",
                                      {"seconds": 1, "name": "Speech pause alarm"}, 201)
                        stream_alarm_started = int(line.split("(", 1)[1].split(")", 1)[0])
                    else:
                        server.release_stream.set()
                if "Voice turn complete" in line and not played:
                    assert stream_alarm_started is None, "The reply ended before the timer alarm"
                    source = ("wake_test" if args.wake_test else "wake" if args.wake_audio
                              else "manual_test" if args.manual_test else "push_to_talk")
                    report = panel.verify_voice("follow_up" if dialogue_started else source, args.stream_speech)
                    if args.dialogue and not dialogue_started:
                        dialogue_started = True
                        server.release_stream.clear()
                        panel.wait_voice(lambda state: state["last_voice"]["source"] == "follow_up" or
                                         (state["active"] and state["active"]["stage"] != "awaiting_speech"),
                                         timeout=30)
                        continue
                    if dialogue_started:
                        assert panel.request("/api/state")["active"]["stage"] == "awaiting_speech"
                        panel.wait_voice(lambda state: not state["active"], timeout=30)
                        assert panel.request("/api/state")["last_voice"] == report
                        print("Verified the spoken follow-up, its context and the silent timeout")
                    else:
                        assert panel.request("/api/state")["active"] is None
                    panel.request("/api/chat/clear", "POST", {}, 204)
                    assert panel.request("/api/state")["messages"] == []
                    if args.dataset_package and args.wake_audio:
                        panel.stop_dataset(active_model)
                    if args.timers_package:
                        panel.exercise_timers(args.timers_package, args.timer_voice_tool)
                    if args.panel_settings:
                        panel.mute_microphone_and_submit_stale_settings()
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
        expected["wake_runtimes"].extend(["microwakeword-tflite", "livekit-v2-micro-tflite"])
    assert capabilities == expected, capabilities
    assert psram_ready, "The ESP32-S3 did not report 8 MiB of PSRAM"
    assert started and played, "Firmware did not finish a voice turn"
    assert persisted, "Panel configuration was not verified after restart"
    if args.provision_wifi:
        assert provision_steps == 2, "Wi-Fi provisioning and retry were not exercised"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--emulator", default="esp-emu")
    parser.add_argument("--emulator-timeout", type=int, help="Override the wall-clock deadline in seconds")
    parser.add_argument("--examples", type=Path, required=True)
    parser.add_argument("--firmware", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--wake-build", action="store_true", help="Expect a wake-enabled firmware image")
    parser.add_argument("--no-plugins", action="store_true", help="Expect a firmware built without WASM")
    parser.add_argument("--timers-package", type=Path, help="Exercise the real Timers WASM package")
    parser.add_argument("--timer-voice-tool", action="store_true", help="Let the fixture invoke Timers during the voice turn")
    parser.add_argument("--dataset-package", type=Path, help="Check the dataset quota and collect a clip when given wake audio")
    parser.add_argument("--provider-package", type=Path, help="Use the real OpenAI-compatible WASM package")
    activation = parser.add_mutually_exclusive_group()
    activation.add_argument("--wake-audio", type=Path, help="Use a 16 kHz mono PCM WAV containing the wake word")
    activation.add_argument("--manual-test", action="store_true", help="Start the microphone check from the panel")
    activation.add_argument("--dialogue", action="store_true", help="Start with the button, speak a follow-up and wait for the conversation to close")
    parser.add_argument("--provision-wifi", action="store_true", help="Start with blank NVS and verify Wi-Fi setup")
    parser.add_argument("--wifi-during-alarm", action="store_true", help="Verify timer audio while changing Wi-Fi")
    parser.add_argument("--update-image", type=Path, help="Verify an OTA install and reboot using this app.bin")
    parser.add_argument("--interrupt-update", action="store_true", help="Reset before OTA verification and check rollback")
    parser.add_argument("--stream-speech", action="store_true", help="Verify playback starts before the whole audio response arrives")
    parser.add_argument("--alarm-during-stream", action="store_true", help="Play a timer alarm while the speech provider stalls")
    parser.add_argument("--panel-settings", action="store_true", help="Check the panel contract, persisted settings and protected diagnostics")
    parser.add_argument("--wake-test", action="store_true", help="Run the wake-word check with continuous listening paused")
    parser.add_argument("--wake-catalog-model", type=Path, help="Serve and install a Micro INT8 fixture model from a test catalog")
    parser.add_argument("--voice-lifecycle", action="store_true", help="Cancel recording, WASM provider calls and playback")
    args = parser.parse_args()
    if args.emulator_timeout is not None and args.emulator_timeout <= 0:
        parser.error("--emulator-timeout must be positive")
    if args.interrupt_update and not args.update_image:
        parser.error("--interrupt-update requires --update-image")
    if (args.timers_package or args.dataset_package or args.provider_package) and args.no_plugins:
        parser.error("Plugin packages require plugin support")
    if args.wake_test and not args.wake_audio:
        parser.error("--wake-test requires --wake-audio")
    if args.timer_voice_tool and not args.timers_package:
        parser.error("--timer-voice-tool requires --timers-package")
    if args.wifi_during_alarm and not args.timers_package:
        parser.error("--wifi-during-alarm requires --timers-package")
    if args.alarm_during_stream and (not args.timers_package or not args.stream_speech or args.voice_lifecycle):
        parser.error("--alarm-during-stream requires Timers and streamed speech without --voice-lifecycle")
    if args.voice_lifecycle and (not args.provider_package or not args.stream_speech or args.manual_test or args.dialogue
                                 or args.wake_audio or args.timer_voice_tool):
        parser.error("--voice-lifecycle requires a streaming WASM provider and push-to-talk")
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
        server.dialogue = args.dialogue
        server.streaming = args.stream_speech
        server.stream_timeout = 25 if args.alarm_during_stream else 8
        server.tools = args.timer_voice_tool
        server.plugin_provider = bool(args.provider_package)
        server.panel_settings = args.panel_settings
        server.wake_catalog = wake_catalog(args.wake_catalog_model) if args.wake_catalog_model else None
        server.wake_downloads = 0
        server.wake_catalog_requests = 0
        server.catalog_request_started = Event()
        server.release_catalog = Event()
        server.release_stream = Event()
        server.release_chat = Event()
        server.hold_transcription = None
        server.transcription_started = Event()
        server.release_transcription = Event()
        server.cancel_stream = False
        server.stream_cancelled = Event()
        server.playback_started = Event()
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            run_emulator(args, flash, board, output, server)
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
        if args.dialogue:
            expected.extend(["/v1/audio/transcriptions", "/v1/chat/completions", "/v1/audio/speech"])
        if args.provider_package:
            expected.insert(0, "/v1/models")
        if args.voice_lifecycle:
            expected[2:2] = [
                "/v1/audio/transcriptions", "/v1/audio/transcriptions",
                "/v1/chat/completions", "/v1/audio/speech",
            ]
        assert server.requests == expected, server.requests
        assert server.wake_downloads == (2 if args.wake_catalog_model else 0)
    with wave.open(str(output)) as wav:
        assert (wav.getnchannels(), wav.getsampwidth(), wav.getframerate()) == (1, 2, 16000)
        frames = wav.getnframes()
        samples = struct.unpack(f"<{frames}h", wav.readframes(frames))
        check_amplifier(samples, 16000, args.timers_package, args.voice_lifecycle, args.alarm_during_stream)
    print(f"Verified panel setup, chat, diagnostics and the 660 Hz amplifier response in {output}")


if __name__ == "__main__":
    main()
