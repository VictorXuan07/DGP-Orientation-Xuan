"""Linux real-process checks; build both Rust binaries before running."""

import argparse
import codecs
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import socket
import subprocess
import tempfile
import termios
import time
import unittest
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
SERVER = ROOT / "target/debug/rm-server-async"
CLIENT = ROOT.parent / "client-sync/target/debug/rm-client-sync"
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def http(url, method, path, body=None, token=""):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    request = urllib.request.Request(
        url + path, method=method, headers=headers,
        data=None if body is None else json.dumps(body).encode(),
    )
    try:
        response = HTTP.open(request, timeout=2)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        data = response.read()
        return response.status, json.loads(data) if data else None


class Server:
    def __init__(self):
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            self.address = "127.0.0.1:" + str(reservation.getsockname()[1])
        self.url = "http://" + self.address
        self.process = None
        self.log = tempfile.TemporaryFile()

    def start(self):
        self.process = subprocess.Popen(
            [str(SERVER), "--address", self.address],
            stdout=self.log, stderr=self.log,
        )
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                break
            try:
                if http(self.url, "GET", "/ping") == (200, {"data": "pong"}):
                    return self
            except (OSError, urllib.error.URLError):
                pass
            time.sleep(0.01)
        self.log.seek(0)
        raise AssertionError("server not ready: " + self.log.read().decode())

    def stop(self):
        if self.process is not None and self.process.poll() is None:
            self.process.send_signal(signal.SIGINT)
            try:
                code = self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
                raise AssertionError("server did not exit after SIGINT")
            if code != 0:
                raise AssertionError(f"server shutdown status: {code}")

    def __enter__(self):
        try:
            return self.start()
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, *_):
        try:
            self.stop()
        finally:
            self.log.close()


class Terminal:
    def __init__(self, url):
        self.master, slave = pty.openpty()

        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        self.process = subprocess.Popen(
            [str(CLIENT), "--url", url], stdin=slave, stdout=slave, stderr=slave,
            preexec_fn=controlling_terminal,
        )
        os.close(slave)
        self.buffer = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")()

    def expect(self, pattern):
        deadline = time.monotonic() + 10
        while True:
            match = re.search(pattern, self.buffer)
            if match:
                self.buffer = self.buffer[match.end():]
                return match
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AssertionError(f"waiting for {pattern!r}: {self.buffer!r}")
            if select.select([self.master], [], [], remaining)[0]:
                try:
                    data = os.read(self.master, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    data = b""
                if not data:
                    raise AssertionError(f"terminal closed before {pattern!r}: {self.buffer!r}")
                self.buffer += self.decoder.decode(data)

    def send(self, text):
        os.write(self.master, text.encode())

    def command(self, command, status, name=None, password=None, text=None):
        self.expect(r" / q > ")
        self.send(command + "\n")
        if password is not None:
            self.expect("username: ")
            self.send(name + "\n")
            self.expect("password: ")
            self.send(password + "\n")
        elif name is not None:
            self.expect("name: ")
            self.send(name + "\n")
        if text is not None:
            self.expect("extra '.' prefix.")
            self.send(text)
        result = self.expect(r"(?:^|\n)" + str(status) + r" (\{[^\r\n]*\})")
        return json.loads(result.group(1))

    def quit(self):
        self.expect(r" / q > ")
        self.send("q\n")
        if self.process.wait(timeout=5) != 0:
            raise AssertionError("q did not exit successfully")

    def __enter__(self):
        return self

    def __exit__(self, *_):
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=5)
        os.close(self.master)


class RuntimeTests(unittest.TestCase):
    def test_real_terminal_account_text_lifecycle_and_isolation(self):
        with Server() as server, Terminal(server.url) as alice, Terminal(server.url) as bob:
            for terminal, name in [(alice, "alice"), (bob, "bob")]:
                terminal.command("register", 201, name=name, password="password1")
                terminal.command("register", 409, name=name, password="password1")
                terminal.command("login", 401, name=name, password="wrongpassword")
                login = terminal.command("login", 200, name=name, password="password1")
                self.assertGreater(login["data"]["expires_in"], 0)
                self.assertEqual(terminal.command("list", 200), {"data": []})
            alice.command("put", 200, name="note", text="你好\n..\n.end\n")
            bob.command("put", 200, name="note", text="bob text\n.end\n")
            self.assertEqual(alice.command("get", 200, name="note"), {"data": "你好\n."})
            self.assertEqual(bob.command("get", 200, name="note"), {"data": "bob text"})
            self.assertEqual(alice.command("echo", 200, text=".end\n"), {"data": ""})
            alice.command("put", 200, name="A", text=".end\n")
            self.assertEqual(alice.command("list", 200), {"data": ["A", "note"]})
            alice.command("delete", 200, name="note")
            alice.command("delete", 404, name="note")
            self.assertEqual(bob.command("get", 200, name="note"), {"data": "bob text"})
            alice.command("logout", 200)
            alice.command("list", 401)
            alice.command("login", 200, name="alice", password="password1")
            # A login outside this client replaces its saved token.
            self.assertEqual(http(server.url, "POST", "/sessions", {"username": "alice", "password": "password1"})[0], 200)
            alice.command("list", 401)
            alice.command("login", 200, name="alice", password="password1")
            alice.command("delete-user", 200)
            alice.command("list", 401)
            alice.command("register", 201, name="alice", password="password1")
            alice.command("login", 200, name="alice", password="password1")
            self.assertEqual(alice.command("list", 200), {"data": []})
            self.assertEqual(bob.command("list", 200), {"data": ["note"]})
            alice.quit()
            bob.quit()

    def test_sigint_shutdown_and_restart_clear_all_state(self):
        account = {"username": "restart", "password": "password1"}
        with Server() as server:
            self.assertEqual(http(server.url, "POST", "/users", account)[0], 201)
            token = http(server.url, "POST", "/sessions", account)[1]["data"]["token"]
            self.assertEqual(http(server.url, "PUT", "/texts/note", {"text": "lost"}, token)[0], 200)
            with Terminal(server.url) as terminal:
                terminal.expect(r" / q > ")
                terminal.send("\x03")
                self.assertEqual(terminal.process.wait(timeout=5), -signal.SIGINT)
            server.stop()
            host, port = server.address.split(":")
            with socket.socket() as connection:
                connection.settimeout(1)
                self.assertNotEqual(connection.connect_ex((host, int(port))), 0)
            server.start()
            self.assertEqual(http(server.url, "GET", "/texts", token=token)[0], 401)
            self.assertEqual(http(server.url, "POST", "/sessions", account)[0], 401)
            self.assertEqual(http(server.url, "POST", "/users", account)[0], 201)
            next_token = http(server.url, "POST", "/sessions", account)[1]["data"]["token"]
            self.assertEqual(http(server.url, "GET", "/texts", token=next_token), (200, {"data": []}))

    def test_failed_request_keeps_other_user_data_and_server_alive(self):
        with Server() as server:
            account = {"username": "keep", "password": "password1"}
            self.assertEqual(http(server.url, "POST", "/users", account)[0], 201)
            token = http(server.url, "POST", "/sessions", account)[1]["data"]["token"]
            self.assertEqual(http(server.url, "PUT", "/texts/note", {"text": "keep"}, token)[0], 200)
            host, port = server.address.split(":")
            with socket.create_connection((host, int(port)), timeout=2) as connection:
                connection.sendall(b"POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 3\r\nConnection: close\r\n\r\n{x}")
                self.assertIn(b"400", connection.recv(4096).split(b"\r\n")[0])
            # An abruptly disconnected body must not affect subsequent requests.
            with socket.create_connection((host, int(port)), timeout=2) as connection:
                connection.sendall(b"POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\n\r\n{")
            self.assertEqual(http(server.url, "GET", "/texts/note", token=token), (200, {"data": "keep"}))
            self.assertEqual(http(server.url, "GET", "/ping"), (200, {"data": "pong"}))

    def test_invalid_server_arguments_have_errors(self):
        for arguments in [["--address", "invalid"], ["--token-ttl-seconds", "0"]]:
            result = subprocess.run([str(SERVER), *arguments], capture_output=True, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"error:", result.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, default=SERVER)
    parser.add_argument("--client", type=Path, default=CLIENT)
    args, remaining = parser.parse_known_args()
    SERVER, CLIENT = args.server.resolve(), args.client.resolve()
    unittest.main(argv=[__file__, *remaining])
