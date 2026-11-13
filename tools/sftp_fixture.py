"""Loopback-only SSH/SFTP fixture for integration tests; never a production server.

Install: python -m pip install --target artifacts/python paramiko
Run: python tools/sftp_fixture.py --ready artifacts/fixture.json
The test account is arcnix / arcnix-test. Files stay under an isolated temp root.
"""
from __future__ import annotations

import argparse
import errno
import json
import os
from pathlib import Path
import socket
import sys
import tempfile
import threading
import time
import shlex
import hashlib
import base64

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "artifacts" / "python"))
import paramiko
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey


class Authentication(paramiko.ServerInterface):
    def __init__(self, client_key, transport, audit_auth):
        super().__init__()
        self.client_key = client_key
        self.transport = transport
        self.audit_auth = audit_auth
        self.sftp_enabled = True
        self.nginx_installed = False
        self.metrics_sample = 0
        self.username = ""
        self.terminal_size = (100, 30)

    def check_auth_password(self, username, password):
        self.username = username
        self.audit_auth(username, "password")
        self.sftp_enabled = username != "arcnix-nosftp"
        return paramiko.AUTH_SUCCESSFUL if username in ("arcnix", "arcnix-nosftp", "arcnix-no-terminal") and password == "arcnix-test" else paramiko.AUTH_FAILED

    def get_allowed_auths(self, username):
        return "password,publickey"

    def check_auth_publickey(self, username, key):
        self.username = username
        self.audit_auth(username, "publickey")
        self.sftp_enabled = username != "arcnix-nosftp"
        return paramiko.AUTH_SUCCESSFUL if username in ("arcnix", "arcnix-nosftp", "arcnix-no-terminal") and key in self.client_key else paramiko.AUTH_FAILED

    def check_channel_pty_request(self, channel, term, width, height, pixelwidth, pixelheight, modes):
        self.terminal_size = (width, height)
        return self.username != "arcnix-no-terminal" and term == b"xterm-256color"

    def check_channel_window_change_request(self, channel, width, height, pixelwidth, pixelheight):
        self.terminal_size = (width, height)
        return True

    def check_channel_shell_request(self, channel):
        # A deterministic SSH PTY protocol peer, never a host OS shell.
        def interactive():
            try:
                channel.sendall(b"\x1b[?2004h\x1b[32mArcnix terminal fixture\x1b[0m\r\n$ ")
                line = bytearray()
                while True:
                    data = channel.recv(8192)
                    if not data:
                        return
                    data = data.replace(b"\x1b[200~", b"").replace(b"\x1b[201~", b"")
                    for value in data:
                        if value == 3:
                            line.clear()
                            channel.sendall(b"^C\r\n$ ")
                        elif value in (8, 127):
                            if line:
                                line.pop()
                                channel.sendall(b"\b \b")
                        elif value in (10, 13):
                            command = bytes(line).decode("utf-8", errors="replace")
                            line.clear()
                            channel.sendall(b"\r\n")
                            if command == "exit":
                                channel.send_exit_status(0)
                                channel.close()
                                return
                            if command == "pwd":
                                channel.sendall(b"/fixture\r\n")
                            elif command == "size":
                                channel.sendall((str(self.terminal_size[0]) + "x" + str(self.terminal_size[1]) + "\r\n").encode())
                            elif command == "ansi":
                                channel.sendall(b"\x1b[31mRED\x1b[0m\r\n")
                            elif command == "osc52":
                                channel.sendall(b"\x1b]52;c;YmFkLWNsaXBib2FyZA==\x07")
                            elif command == "unicode":
                                encoded = "中文终端\r\n".encode()
                                channel.sendall(encoded[:2])
                                channel.sendall(encoded[2:])
                            elif command == "burst":
                                for index in range(5000):
                                    channel.sendall((str(index) + " " + "terminal-output " * 8 + "\r\n").encode())
                                channel.sendall(b"BURST COMPLETE\r\n")
                            elif command.startswith("echo "):
                                channel.sendall(command[5:].encode() + b"\r\n")
                            elif command.startswith("checksum "):
                                channel.sendall(hashlib.sha256(command[9:].encode()).hexdigest().encode() + b"\r\n")
                            elif command:
                                channel.sendall(b"Fixture command received\r\n")
                            channel.sendall(b"$ ")
                        else:
                            if len(line) < 262144:
                                line.append(value)
                                channel.sendall(bytes([value]))
            except (OSError, EOFError, paramiko.SSHException):
                pass
        threading.Thread(target=interactive, daemon=True).start()
        return True

    def check_channel_subsystem_request(self, channel, name):
        if name == "sftp" and not self.sftp_enabled:
            return False
        return super().check_channel_subsystem_request(channel, name)

    def check_channel_request(self, kind, channel_id):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_exec_request(self, channel, command):
        def reply():
            # Deliberately does not execute arbitrary commands on the developer's machine.
            text = command.decode()
            if text.strip() == "fixture-stream":
                channel.sendall(b"first stdout\n")
                time.sleep(.15)
                channel.sendall_stderr(b"first stderr\n")
                time.sleep(.15)
                channel.sendall(b"last stdout\n")
                channel.send_exit_status(9)
            elif text.strip() == "fixture-stream-large":
                for _ in range(100):
                    channel.sendall(b"o" * 8192)
                    channel.sendall_stderr(b"e" * 8192)
                channel.sendall(b"stdout-end\n")
                channel.sendall_stderr(b"stderr-end\n")
                channel.send_exit_status(0)
            elif text.strip() == "fixture-stream-timeout":
                time.sleep(2)
                if channel.closed:
                    return
                channel.send_exit_status(0)
            elif text.strip() == "fixture-stream-input":
                data = bytearray()
                while True:
                    chunk = channel.recv(1024)
                    if not chunk:
                        break
                    data.extend(chunk)
                channel.sendall(b"stdin accepted\n" if data == b"fixture-private-input\n" else b"stdin rejected\n")
                channel.send_exit_status(0 if data == b"fixture-private-input\n" else 1)
            elif text.startswith("# arcnix:transfer:replace"):
                # Emulate only the fixed replacement adapter within the fixture
                # sandbox. Never pass commands to a shell on the test machine.
                try:
                    parts = shlex.split(text.splitlines()[-1])
                    assert parts[-5:-2] == ["mv", "-fT", "--"]
                    files = Files(self, root=self.fixture_root)
                    source, target = files.path(parts[-2]), files.path(parts[-1])
                    assert source.is_file() and target.is_file() and not source.is_symlink() and not target.is_symlink()
                    if target.name.startswith("fixture-replace-denied-"):
                        raise PermissionError("Fixture replacement denied")
                    os.replace(source, target)
                    Files.moved(source, target)
                    channel.send_exit_status(0)
                except (OSError, AssertionError, ValueError) as error:
                    channel.sendall_stderr((str(error) + "\n").encode())
                    channel.send_exit_status(1)
            elif text.startswith("# arcnix:metrics:sample"):
                self.metrics_sample += 1
                count = self.metrics_sample
                payload = f"""ARCNIX_METRICS_V1
ARCNIX_CPU_BEGIN
cpu  {100 + count * 60} 20 30 {400 + count * 140} 10 5 5 0 20 0
cpu0 50 10 15 200 5 2 3 0 10 0
cpu1 50 10 15 200 5 3 2 0 10 0
ARCNIX_CPU_END
ARCNIX_MEMORY_BEGIN
MemTotal: 1048576 kB
MemAvailable: 524288 kB
SwapTotal: 262144 kB
SwapFree: 196608 kB
ARCNIX_MEMORY_END
ARCNIX_LOAD_BEGIN
0.25 0.50 0.75 1/100 321
ARCNIX_LOAD_END
ARCNIX_UPTIME_BEGIN
{1000 + count * 3}.00 1800.00
ARCNIX_UPTIME_END
ARCNIX_NETWORK_BEGIN
Inter-| Receive | Transmit
 face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed
lo: 10 1 0 0 0 0 0 0 10 1 0 0 0 0 0 0
eth0: {10000 + count * 3072} 10 0 0 0 0 0 0 {20000 + count * 6144} 20 0 0 0 0 0 0
ARCNIX_NETWORK_END
ARCNIX_DISK_BEGIN
Filesystem 1024-blocks Used Available Capacity Mounted on
/dev/root 1048576 262144 786432 25% /
/dev/data 2097152 1048576 1048576 50% /srv/my data
ARCNIX_DISK_END
ARCNIX_METRICS_END
"""
                channel.sendall(payload.encode())
                channel.send_exit_status(0)
            elif text.startswith("# arcnix:logs:discover"):
                channel.sendall(b"ARCNIX_LOGS_V1\njournal\nsyslog\nauth\n")
                channel.send_exit_status(0)
            elif text.startswith("# arcnix:logs:read"):
                if "denied.service" in text:
                    channel.sendall_stderr(b"Permission denied reading system journal\n")
                    channel.send_exit_status(1)
                elif "empty.service" in text:
                    channel.send_exit_status(0)
                elif "journalctl" in text:
                    records = [dict(__REALTIME_TIMESTAMP="1794499200000000", _SYSTEMD_UNIT="ssh.service", PRIORITY="6", MESSAGE="Accepted publickey for arcnix"),
                               dict(__REALTIME_TIMESTAMP="1794499260000000", _SYSTEMD_UNIT="nginx.service", PRIORITY="4", MESSAGE="Fixture service warning\nSecond line preserved")]
                    if "ssh.service" in text:
                        records = records[:1]
                    if "--priority=3" in text or "-p 3" in text:
                        records = []
                    channel.sendall(("\n".join(json.dumps(record) for record in records) + "\n").encode())
                    channel.send_exit_status(0)
                else:
                    channel.sendall(b"Nov 12 08:00:00 fixture sshd[321]: Accepted publickey for arcnix\nNov 12 08:01:00 fixture kernel: Fixture file log entry\n")
                    channel.send_exit_status(0)
            elif text.startswith("# arcnix:packages:"):
                marker = text.splitlines()[0]
                if marker.endswith(":detect"):
                    channel.sendall(b'ARCNIX_OS_BEGIN\nPRETTY_NAME="Ubuntu 24.04 LTS"\nARCNIX_OS_END\nARCNIX_MANAGER\tapt\nARCNIX_UID\t1000\nARCNIX_SUDO\t1\n')
                elif marker.endswith(":search"):
                    channel.sendall(b"nginx - web server\ncurl - transfer tool\n")
                elif marker.endswith(":installed"):
                    channel.sendall(b"curl\t8.5\tii \ttransfer tool\n")
                    if self.nginx_installed:
                        channel.sendall(b"nginx\t1.24\tii \tweb server\n")
                elif marker.endswith(":verify"):
                    channel.sendall(b"Package: nginx\nVersion: 1.24\nArchitecture: amd64\n\nPackage: curl\nVersion: 8.5\nArchitecture: amd64\n")
                elif marker.endswith(":preview"):
                    channel.sendall(b"Reading package lists...\nInst nginx (1.24 Ubuntu [amd64])\nConf nginx (1.24 Ubuntu [amd64])\n")
                elif marker.endswith(":install") or marker.endswith(":update"):
                    channel.sendall(b"Reading package lists...\n")
                    time.sleep(.35)
                    channel.sendall(b"Processing package operation...\n")
                    time.sleep(.35)
                    channel.sendall(b"Package operation completed (isolated fixture).\n")
                    if marker.endswith(":install"):
                        self.nginx_installed = True
                else:
                    channel.sendall_stderr(b"Unsupported fixture package marker\n")
                    channel.send_exit_status(1)
                    channel.close()
                    return
                channel.send_exit_status(0)
            elif text.strip() == "fixture-output":
                channel.sendall(b"fixture stdout\n")
                channel.sendall_stderr(b"fixture stderr\n")
                channel.send_exit_status(7)
            else:
                channel.sendall(b"Arcnix isolated SSH fixture\n")
                channel.send_exit_status(0)
            channel.close()
        threading.Thread(target=reply, daemon=True).start()
        return True


class DelayedHandle(paramiko.SFTPHandle):
    """Optional latency for visible transfer progress in loopback UI tests."""
    delay = 0
    def read(self, offset, length):
        if self.delay: time.sleep(self.delay)
        return super().read(offset, length)
    def write(self, offset, data):
        if self.delay: time.sleep(self.delay)
        return super().write(offset, data)


class Files(paramiko.SFTPServerInterface):
    # Windows cannot represent Linux mode bits; retain them for protocol tests.
    modes = {}

    @classmethod
    def attributes(cls, path):
        attributes = paramiko.SFTPAttributes.from_stat(path.lstat())
        if str(path) in cls.modes:
            attributes.st_mode = (attributes.st_mode & ~0o7777) | cls.modes[str(path)]
        return attributes

    @classmethod
    def moved(cls, source, target):
        mode = cls.modes.pop(str(source), None)
        cls.modes.pop(str(target), None)
        if mode is not None:
            cls.modes[str(target)] = mode

    def __init__(self, server, *args, root, **kwargs):
        super().__init__(server, *args, **kwargs)
        self.root = Path(root).resolve()
        self.transport = server.transport

    def path(self, path):
        target = (self.root / path.lstrip("/")).resolve()
        if not target.is_relative_to(self.root):
            raise PermissionError(errno.EACCES, "Outside test root")
        return target

    def canonicalize(self, path):
        if path == "/__fixture_disconnect__":
            self.transport.close()
            return "/"
        try:
            return "/" + self.path(path).relative_to(self.root).as_posix().removeprefix(".")
        except OSError:
            return "/"

    @staticmethod
    def error(exc):
        return paramiko.SFTPServer.convert_errno(exc.errno or errno.EIO)

    def list_folder(self, path):
        try:
            if path == "/__fixture_delay__":
                time.sleep(1.5)
                path = "/"
            rows = []
            for child in self.path(path).iterdir():
                try:
                    item = self.attributes(child)
                except FileNotFoundError:
                    # Parallel transfers may publish/remove a staging file
                    # between enumeration and lstat; the directory still exists.
                    continue
                item.filename = child.name
                rows.append(item)
            return rows
        except OSError as exc:
            return self.error(exc)

    def stat(self, path):
        try:
            return self.attributes(self.path(path))
        except OSError as exc:
            return self.error(exc)

    def lstat(self, path):
        try:
            return self.attributes(self.path(path))
        except OSError as exc:
            return self.error(exc)

    def open(self, path, flags, attr):
        try:
            fd = os.open(self.path(path), flags | getattr(os, "O_BINARY", 0), attr.st_mode or 0o600)
            if flags & os.O_CREAT:
                self.modes[str(self.path(path))] = (attr.st_mode or 0o600) & 0o7777
            mode = "r+b" if flags & os.O_RDWR else ("wb" if flags & os.O_WRONLY else "rb")
            stream = os.fdopen(fd, mode)
            handle = DelayedHandle(flags)
            if not flags & os.O_WRONLY:
                handle.readfile = stream
            if flags & (os.O_WRONLY | os.O_RDWR):
                handle.writefile = stream
            return handle
        except OSError as exc:
            return self.error(exc)

    def remove(self, path):
        try:
            self.path(path).unlink()
            self.modes.pop(str(self.path(path)), None)
            return paramiko.SFTP_OK
        except OSError as exc:
            return self.error(exc)

    def mkdir(self, path, attr):
        try:
            self.path(path).mkdir(mode=attr.st_mode or 0o755)
            return paramiko.SFTP_OK
        except OSError as exc:
            return self.error(exc)

    def rmdir(self, path):
        try:
            self.path(path).rmdir()
            return paramiko.SFTP_OK
        except OSError as exc:
            return self.error(exc)

    def rename(self, oldpath, newpath):
        try:
            target = self.path(newpath)
            if target.exists():
                return paramiko.SFTP_FAILURE
            self.path(oldpath).rename(target)
            self.moved(self.path(oldpath), target)
            return paramiko.SFTP_OK
        except OSError as exc:
            return self.error(exc)

    def posix_rename(self, oldpath, newpath):
        try:
            source = self.path(oldpath)
            target = self.path(newpath)
            if source.is_dir() or target.is_dir() or target.is_symlink():
                return paramiko.SFTP_FAILURE
            os.replace(source, target)
            self.moved(source, target)
            return paramiko.SFTP_OK
        except OSError as exc:
            return self.error(exc)

    def chattr(self, path, attributes):
        if "fixture-mode-denied-" in path:
            return paramiko.SFTP_PERMISSION_DENIED
        try:
            target = self.path(path)
            target.stat()
            if attributes.st_mode is not None:
                self.modes[str(target)] = attributes.st_mode & 0o7777
            return paramiko.SFTP_OK
        except OSError as exc:
            return self.error(exc)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--ready", type=Path, required=True)
    parser.add_argument("--io-delay", type=float, default=0, help="Test-only delay per SFTP read/write (0–0.1 seconds)")
    args = parser.parse_args()
    if not 0 <= args.io_delay <= 0.1: parser.error("io-delay must be between 0 and 0.1")
    DelayedHandle.delay = args.io_delay
    host_key = paramiko.RSAKey.generate(2048)
    transports = []
    with tempfile.TemporaryDirectory(prefix="arcnix-sftp-fixture-") as directory:
        root = Path(directory) / "files"
        root.mkdir()
        client_key = paramiko.RSAKey.generate(2048)
        client_key_path = Path(directory) / "client-key.pem"
        client_key.write_private_key_file(str(client_key_path))
        ed25519 = Ed25519PrivateKey.generate()
        ed25519_path = Path(directory) / "client-ed25519"
        ed25519_path.write_bytes(ed25519.private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.OpenSSH,
            serialization.NoEncryption()))
        encrypted_path = Path(directory) / "client-ed25519-encrypted"
        encrypted_path.write_bytes(ed25519.private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.OpenSSH,
            serialization.BestAvailableEncryption(b"arcnix-test-key-passphrase")))
        untrusted_path = Path(directory) / "untrusted-ed25519"
        untrusted_path.write_bytes(Ed25519PrivateKey.generate().private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.OpenSSH,
            serialization.NoEncryption()))
        accepted_keys = (client_key, paramiko.Ed25519Key.from_private_key_file(str(ed25519_path)))
        # Publish two independent host keys so tests reproduce a backend upgrade
        # changing its default algorithm without changing the server's RSA key.
        host_ed25519_path = Path(directory) / "host-ed25519"
        host_ed25519_path.write_bytes(Ed25519PrivateKey.generate().private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.OpenSSH,
            serialization.NoEncryption()))
        host_ed25519 = paramiko.Ed25519Key.from_private_key_file(str(host_ed25519_path))
        auth_audit = Path(directory) / "auth-audit.jsonl"
        auth_audit.touch()
        audit_lock = threading.Lock()
        def audit_auth(username, method):
            with audit_lock, auth_audit.open("a", encoding="utf-8") as output:
                output.write(json.dumps({"username": username, "method": method}) + "\n")
        rsa_fingerprint = "SHA256:" + base64.b64encode(hashlib.sha256(host_key.asbytes()).digest()).decode().rstrip("=")
        (root / "welcome.txt").write_text("Arcnix SFTP fixture\n", encoding="utf-8")
        (root / "中文 空格.txt").write_text("远程文件测试\n", encoding="utf-8")
        (root / "folder").mkdir()
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", args.port))
            listener.listen(16)
            listener.settimeout(1)
            args.ready.parent.mkdir(parents=True, exist_ok=True)
            args.ready.write_text(json.dumps({"address": "127.0.0.1:" + str(listener.getsockname()[1]), "root": str(root), "client_key": str(client_key_path), "client_key_ed25519": str(ed25519_path), "client_key_ed25519_encrypted": str(encrypted_path), "client_key_untrusted": str(untrusted_path), "host_key_rsa_fingerprint": rsa_fingerprint, "auth_audit": str(auth_audit)}), encoding="utf-8")
            print("Fixture ready on " + str(listener.getsockname()), flush=True)
            try:
                while True:
                    try:
                        peer, _ = listener.accept()
                    except socket.timeout:
                        continue
                    def serve(peer=peer):
                        transport = paramiko.Transport(peer)
                        # Paramiko has no system DH moduli on Windows; avoid its
                        # group-exchange fallback and negotiate fixed modern KEX.
                        transport.get_security_options().kex = (
                            "ecdh-sha2-nistp256", "ecdh-sha2-nistp384",
                            "diffie-hellman-group14-sha256",
                        )
                        transports.append(transport)
                        transport.add_server_key(host_key)
                        transport.add_server_key(host_ed25519)
                        transport.set_subsystem_handler("sftp", paramiko.SFTPServer, Files, root=root)
                        try:
                            authentication = Authentication(accepted_keys, transport, audit_auth)
                            authentication.fixture_root = root
                            transport.start_server(server=authentication)
                            while transport.is_active():
                                time.sleep(0.1)
                        except (EOFError, paramiko.SSHException):
                            pass
                        finally:
                            transport.close()
                    threading.Thread(target=serve, daemon=True).start()
            except KeyboardInterrupt:
                pass
            finally:
                for transport in transports:
                    transport.close()
                args.ready.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
